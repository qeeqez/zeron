//! Per-thread git worktrees — `default_workspace: worktree` gives each new
//! thread its own checkout under `<root>/.worktrees/` so concurrent agent
//! turns never share a working tree. Worktrees are created detached at
//! HEAD (no branch to clean up) and removed when their chat is deleted.

use std::path::{Path, PathBuf};

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

/// Run `git` in `dir`; stdout on success, stderr text on failure — unlike
/// `crate::git::git`, callers need the error detail for user-facing notes.
fn git_err(dir: &Path, args: &[&str]) -> Result<String, String> {
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

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "worktree_tests.rs"]
mod worktree_tests;
