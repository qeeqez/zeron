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

/// Remove the chat's worktree checkout under `root`, if it has one —
/// the single cleanup entry point for delete/clear/retention paths.
pub(crate) fn remove_for(root: &Path, chat: &crate::model::Chat) {
    if chat.worktree {
        remove(root, Path::new(&chat.workdir));
    }
}

/// Remove every chat's worktree under `root` — the clear-all path.
pub(crate) fn remove_all(root: &Path, chats: &[crate::model::Chat]) {
    for chat in chats {
        remove_for(root, chat);
    }
}

/// Remove a thread's worktree. `git worktree remove --force` handles dirty
/// trees; when the dir is already gone (or git fails) the entry is pruned
/// so `git worktree list` stays clean.
pub(crate) fn remove(root: &Path, path: &Path) {
    let arg = path.to_string_lossy().into_owned();
    if git_err(root, &["worktree", "remove", "--force", &arg]).is_err() {
        let _ = std::fs::remove_dir_all(path);
        let _ = git_err(root, &["worktree", "prune"]);
    }
}

/// One live worktree dir under `.worktrees/` plus the title of the chat
/// that owns it — `None` marks an orphan (no chat's `workdir` points at
/// the dir), the only entries the settings list may delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeInfo {
    pub path: PathBuf,
    pub chat_title: Option<String>,
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
            WorktreeInfo { path, chat_title }
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
            &format!("Delete worktree “{name}”?"),
            Some("No thread uses this checkout. This cannot be undone."),
            &[PromptButton::ok("Delete"), PromptButton::cancel("Cancel")],
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
