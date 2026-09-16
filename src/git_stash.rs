//! Stash actions — `stash_list` plus the push/pop/apply/drop ops the Changes
//! panel's stash section runs. Split from `git.rs` for the SLOC cap;
//! re-exported there so callers keep using `crate::git::stash_list` etc.

use std::path::Path;

/// One stash entry as the Changes panel's stash list sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    /// Reflog selector (`stash@{0}`) — the row label and the argument
    /// pop/apply/drop pass back to git.
    pub name: String,
    /// Stash subject (`%gs`): "WIP on main: …" for an auto message, or
    /// "On main: <msg>" when `stash push -m` supplied one.
    pub message: String,
    /// Relative committer time (`%cr` — "2 hours ago").
    pub rel_time: String,
}

/// Stash entries under `dir`, newest first. Empty on non-repo dirs and when
/// nothing is stashed — `git stash list` prints nothing for an empty reflog.
pub(crate) fn stash_list(dir: &Path) -> Vec<StashEntry> {
    let out = super::git(dir, &["stash", "list", "--format=%gd%x00%gs%x00%cr"]);
    out.map(|o| crate::git_parse::parse_stash_list(&o)).unwrap_or_default()
}

/// `git stash push -u -m <message>` — stash tracked and untracked changes,
/// leaving a clean worktree. On a clean tree git exits 0 with "No local
/// changes to save"; that text is the note, not an error.
pub(crate) fn stash_push(dir: &Path, message: &str) -> Result<String, String> {
    super::git_env(dir, &["stash", "push", "-u", "-m", message], &[]).map(|out| out.trim().to_string())
}

/// `git stash pop <name>` — apply the stash and drop it on success. A merge
/// conflict exits non-zero, keeps the entry, and its stderr is the note.
pub(crate) fn stash_pop(dir: &Path, name: &str) -> Result<String, String> {
    super::git_env(dir, &["stash", "pop", name], &[]).map(|_| format!("Popped {name}"))
}

/// `git stash apply <name>` — apply the stash but keep it in the list.
pub(crate) fn stash_apply(dir: &Path, name: &str) -> Result<String, String> {
    super::git_env(dir, &["stash", "apply", name], &[]).map(|_| format!("Applied {name}"))
}

/// `git stash drop <name>` — remove the entry without applying it.
pub(crate) fn stash_drop(dir: &Path, name: &str) -> Result<String, String> {
    super::git_env(dir, &["stash", "drop", name], &[]).map(|_| format!("Dropped {name}"))
}
