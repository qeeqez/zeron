//! Per-thread git worktrees — `default_workspace: worktree` gives each new
//! thread its own checkout under `<root>/.worktrees/` so concurrent agent
//! turns never share a working tree. Worktrees are created detached at
//! HEAD (no branch to clean up) and removed when their chat is deleted.

use std::path::{Path, PathBuf};

use gpui_kit::{Context, PromptButton, PromptLevel, Window};

use crate::workspace::Workspace;

/// Where new threads run: the project checkout itself, or a per-thread
/// git worktree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    /// Backend turns run in the project root — the original behavior.
    #[default]
    Checkout,
    /// `new_chat` runs `git worktree add` for the thread and its backend
    /// spawns with that directory as cwd.
    Worktree,
}

impl WorkspaceMode {
    /// All modes, in settings-picker order.
    pub const ALL: [WorkspaceMode; 2] = [WorkspaceMode::Checkout, WorkspaceMode::Worktree];

    /// Stable id stored in settings.json.
    pub fn name(self) -> &'static str {
        match self {
            WorkspaceMode::Checkout => "checkout",
            WorkspaceMode::Worktree => "worktree",
        }
    }

    /// Parse a settings.json value; anything unknown falls back to
    /// Checkout so a stale file can't wedge the picker.
    pub fn from_name(name: &str) -> Self {
        Self::ALL.iter().copied().find(|m| m.name() == name).unwrap_or_default()
    }
}

/// The directory a chat's backend turns run in — its worktree when it has
/// one (and it still exists on disk), the project root otherwise.
pub(crate) fn workdir_for(chat: &crate::model::Chat, root: &Path) -> PathBuf {
    if chat.workdir.is_empty() || (chat.worktree && !Path::new(&chat.workdir).exists()) {
        root.to_path_buf()
    } else {
        PathBuf::from(&chat.workdir)
    }
}

/// Create a detached-HEAD worktree for chat `id` under the project's
/// `.worktrees/` dir, then kick off the project's setup script inside it
/// (see `crate::setup_script` — a no-op when none is configured). Returns
/// the worktree path; `Err` carries git's stderr (not a repo, unborn
/// HEAD, …) for the caller to surface.
pub(crate) fn create(project: &crate::project::Project, chat_id: u64) -> Result<PathBuf, String> {
    let dir = project.worktrees_dir().join(format!("thread-{chat_id}"));
    exclude_worktrees_dir(project.root());
    let path = dir.to_string_lossy().into_owned();
    git_err(project.root(), &["worktree", "add", "--detach", &path])?;
    crate::setup_script::spawn(project, &dir);
    Ok(dir)
}

/// What `remove` did with a worktree dir.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Removal {
    /// The checkout is gone (or was already missing — the registry entry
    /// is pruned either way).
    Removed,
    /// The checkout survives: git refused to remove it (uncommitted work,
    /// a lock). The string is the user-facing reason.
    Kept(String),
}

/// Remove the chat's worktree checkout under `root`, if it has one —
/// the single cleanup entry point for delete/clear/retention paths.
/// `Kept` means the worktree had uncommitted work and stays on disk.
pub(crate) fn remove_for(root: &Path, chat: &crate::model::Chat) -> Removal {
    if chat.worktree { remove(root, Path::new(&chat.workdir)) } else { Removal::Removed }
}

/// Remove every chat's worktree under `root` — the clear-all path.
/// Returns the kept dirs (dirty checkouts stay on disk).
pub(crate) fn remove_all(root: &Path, chats: &[crate::model::Chat]) -> Vec<PathBuf> {
    chats
        .iter()
        .filter_map(|c| match remove_for(root, c) {
            Removal::Kept(_) => Some(PathBuf::from(&c.workdir)),
            Removal::Removed => None,
        })
        .collect()
}

/// Remove a thread's worktree — clean checkouts only. `git worktree
/// remove` (no `--force`) refuses a dirty or locked tree, which is the
/// `Kept` case: the dir and its registry entry stay for the settings
/// list. When the dir is already gone the registry entry is pruned so
/// `git worktree list` stays clean; a leftover dir that isn't a git
/// worktree at all (a half-created checkout) is deleted directly.
pub(crate) fn remove(root: &Path, path: &Path) -> Removal {
    let arg = path.to_string_lossy().into_owned();
    match git_err(root, &["worktree", "remove", &arg]) {
        Ok(_) => Removal::Removed,
        Err(e) if is_git_root(path) => Removal::Kept(kept_reason(&e)),
        Err(_) => {
            // Not a registered worktree — a plain leftover dir. Delete it;
            // `worktree prune` also clears the registry entry when the dir
            // was already missing.
            let _ = std::fs::remove_dir_all(path);
            let _ = git_err(root, &["worktree", "prune"]);
            Removal::Removed
        },
    }
}

/// Drop orphaned thread worktrees under `root` — dirs named `thread-*`
/// that no chat's `workdir` points at (a deleted chat whose cleanup never
/// ran, a crash). Only registered git worktrees are touched, and only
/// clean ones: `remove` keeps a dirty checkout rather than discarding
/// uncommitted work. Returns the kept orphans. Runs at workspace open.
pub(crate) fn prune_orphans(root: &Path, chats: &[crate::model::Chat]) -> Vec<PathBuf> {
    // Clear registry entries whose dirs vanished by other means first.
    let _ = git_err(root, &["worktree", "prune"]);
    list_live(root, chats)
        .into_iter()
        .filter(|w| w.chat_title.is_none())
        .filter(|w| w.path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("thread-")))
        .filter(|w| is_git_root(&w.path))
        .filter(|w| matches!(remove(root, &w.path), Removal::Kept(_)))
        .map(|w| w.path)
        .collect()
}

/// Safe-to-remove check for the settings list: a dir that isn't a git
/// worktree has no tracked state to lose; a real worktree is clean when
/// `git status` reports no modified or untracked files — the same bar
/// `git worktree remove` applies.
pub(crate) fn is_clean(path: &Path) -> bool {
    if !is_git_root(path) {
        return true;
    }
    git_err(path, &["status", "--porcelain"]).is_ok_and(|s| s.trim().is_empty())
}

/// `path` is the root of a git working tree (main checkout or linked
/// worktree) — `rev-parse --show-toplevel` resolves to itself. A plain
/// dir inside a repo reports the PARENT's root instead, so nested repos
/// and worktrees are detected while leftovers are not.
fn is_git_root(path: &Path) -> bool {
    let Ok(me) = path.canonicalize() else { return false };
    git_err(path, &["rev-parse", "--show-toplevel"]).is_ok_and(|top| Path::new(top.trim()).canonicalize().is_ok_and(|top| top == me))
}

/// The `Kept` reason for a refused `worktree remove`: dirty/locked trees
/// read as "uncommitted changes", anything else carries git's first
/// stderr line.
fn kept_reason(stderr: &str) -> String {
    if stderr.contains("modified or untracked") || stderr.contains("locked") {
        "uncommitted changes".to_string()
    } else {
        stderr.lines().next().unwrap_or("unknown reason").to_string()
    }
}

/// One live worktree dir under `.worktrees/` plus the title of the chat
/// that owns it — `None` marks an orphan (no chat's `workdir` points at
/// the dir), the only entries the settings list may delete. `clean` is
/// the safe-to-remove check (`is_clean`): dirty orphans keep their
/// Remove button disabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeInfo {
    pub path: PathBuf,
    pub chat_title: Option<String>,
    pub clean: bool,
}

/// Scan `<root>/.worktrees/` for live per-thread checkouts, matching each
/// dir to the chat whose `workdir` is that path. Orphan dirs — leftovers
/// from a deleted chat or a crashed cleanup — report no title. Sorted by
/// path so the settings list is stable; a missing dir yields an empty vec.
pub(crate) fn list_live(root: &Path, chats: &[crate::model::Chat]) -> Vec<WorktreeInfo> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join(".worktrees"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs.into_iter()
        .map(|path| {
            let chat_title = chats.iter().find(|c| Path::new(&c.workdir) == path).map(|c| c.title.to_string());
            WorktreeInfo { clean: is_clean(&path), path, chat_title }
        })
        .collect()
}

/// Keep `.worktrees/` out of the parent repo's status — the dir lives
/// inside the project so threads stay on the same filesystem, but it must
/// not show up as an untracked entry. `.git/info/exclude` is the local,
/// uncommitted ignore file.
fn exclude_worktrees_dir(root: &Path) {
    let Some(common) = crate::git::git(root, &["rev-parse", "--git-common-dir"]) else { return };
    let gitdir = root.join(common.trim());
    let exclude = gitdir.join("info").join("exclude");
    let mut contents = std::fs::read_to_string(&exclude).unwrap_or_default();
    if contents.lines().any(|l| l.trim() == ".worktrees/") {
        return;
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(".worktrees/\n");
    if let Some(dir) = exclude.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&exclude, contents);
}

/// `git` argv issued by this module, in order — the test seam: the real
/// spawn still runs (worktree tests use real repos), but UI tests assert
/// the recorded argv instead of depending on git's behavior.
#[cfg(test)]
pub(crate) static GIT_ARGV: parking_lot::Mutex<Vec<Vec<String>>> = parking_lot::Mutex::new(Vec::new());

/// Run `git` in `dir`; stdout on success, stderr text on failure — unlike
/// `crate::git::git`, callers need the error detail for user-facing notes.
fn git_err(dir: &Path, args: &[&str]) -> Result<String, String> {
    #[cfg(test)]
    GIT_ARGV.lock().push(args.iter().map(|a| (*a).to_string()).collect());
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
/// The Changes panel's diff base for worktree chats — base resolution and
/// the scratch-index change list — split into `worktree_diff.rs` for the
/// SLOC cap; it uses this module's `git_err` and `merge::merge_base`.
#[path = "worktree_diff.rs"]
pub(crate) mod diff;
/// Merge-back — "Merge into project" applies a worktree chat's delta to
/// the project checkout — split into `worktree_merge.rs` for the SLOC
/// cap; it uses this module's `git_err`, `is_clean` and `remove`.
#[path = "worktree_merge.rs"]
pub(crate) mod merge;

impl Workspace {
    /// Remove an orphan worktree dir — one no chat's `workdir` points at —
    /// behind a native confirm. Ownership is re-checked after the prompt:
    /// a chat created meanwhile keeps its checkout.
    pub(crate) fn remove_orphan_worktree(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.project.root().to_path_buf();
        let owned = || self.chats.iter().any(|c| Path::new(&c.workdir) == path);
        if !path.starts_with(self.project.worktrees_dir()) || owned() {
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Remove worktree “{name}”?"),
            Some("No thread uses this checkout. This cannot be undone."),
            &[PromptButton::ok("Remove"), PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.remove_orphan_worktree_now(root, path, cx));
        })
        .detach();
    }

    /// The confirmed half of `remove_orphan_worktree`: re-check ownership
    /// (the prompt is async — a chat may have claimed the dir meanwhile),
    /// then remove the checkout and re-render the settings list.
    fn remove_orphan_worktree_now(&mut self, root: PathBuf, path: PathBuf, cx: &mut Context<Self>) {
        if self.chats.iter().any(|c| Path::new(&c.workdir) == path) {
            return;
        }
        remove(&root, &path);
        cx.notify();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "worktree_tests.rs"]
mod worktree_tests;

// Same — the header badge + ⋯ menu tests live beside the subsystem.
#[cfg(test)]
#[path = "worktree_ui_tests.rs"]
mod worktree_ui_tests;

// The settings-list tests live beside the subsystem too.
#[cfg(test)]
#[path = "worktree_list_tests.rs"]
mod worktree_list_tests;
