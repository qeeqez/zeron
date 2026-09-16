//! Merge-conflict resolution for the Changes panel: conflict detection
//! (`git diff --name-only --diff-filter=U`), the resolve ops (`git checkout
//! --ours|--theirs` + `git add`, `git add` to mark resolved), and the
//! `Workspace` methods behind the conflicts section's per-file actions —
//! keep ours/theirs, open the file in an editor, mark a hand-edited file
//! resolved. The list is collected by `refresh_changes` in `crate::changes`;
//! rendering lives in `crate::views::changes_conflicts`.

use gpui_kit::*;

use crate::changes::GitOp;
use crate::open_in::PreferredEditor;
use crate::workspace::Workspace;

/// Which side of a merge conflict to keep — the flag `git checkout` takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictSide {
    Ours,
    Theirs,
}

impl ConflictSide {
    fn flag(self) -> &'static str {
        match self {
            Self::Ours => "--ours",
            Self::Theirs => "--theirs",
        }
    }
}

/// Paths still unmerged under `dir` — `git diff --name-only --diff-filter=U`,
/// NUL-separated so names with whitespace survive. Empty outside a merge or
/// rebase conflict and on non-repo dirs.
pub(crate) fn conflicted_files(dir: &std::path::Path) -> Vec<String> {
    crate::git::git(dir, &["diff", "--name-only", "--diff-filter=U", "-z"])
        .map(|raw| crate::git_parse::parse_names(&raw))
        .unwrap_or_default()
}

/// Resolve `path` to one side of the conflict: `git checkout --ours|--theirs
/// -- <path>` rewrites the worktree file, then `git add` marks it resolved.
/// The add runs in the same op so the panel's refresh drops the row.
pub(crate) fn resolve_conflict(dir: &std::path::Path, path: &str, side: ConflictSide) -> Result<String, String> {
    crate::git::git_env(dir, &["checkout", side.flag(), "--", path], &[])?;
    crate::git::stage(dir, path).map(|_| format!("Resolved {path} ({})", side.flag().trim_start_matches('-')))
}

impl Workspace {
    /// Keep one side of the conflict at `git.conflicts[ix]` — `git checkout
    /// --ours|--theirs -- <path>` then `git add` — through `run_git_op` so it
    /// can't interleave with a stage/commit in flight and the panel refreshes
    /// (dropping the row) when it lands.
    pub fn resolve_conflict(&mut self, ix: usize, side: ConflictSide, cx: &mut Context<Self>) {
        let Some(path) = self.git.conflicts.get(ix).cloned() else { return };
        self.run_git_op(GitOp::Resolve(path, side), cx);
    }

    /// `git add <path>` for a conflict the user resolved by hand in an
    /// editor — the row's "Mark resolved" check.
    pub fn mark_conflict_resolved(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(path) = self.git.conflicts.get(ix).cloned() else { return };
        self.run_git_op(GitOp::Stage(path), cx);
    }

    /// Open the conflicted file so its `<<<<<<<` markers can be edited by
    /// hand. `Ask` can't open anything (the file menu shows a picker a bare
    /// button can't), so it falls back to revealing the file in Finder —
    /// same as the `Finder` preference.
    pub fn open_conflict_in_editor(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(path) = self.git.conflicts.get(ix).cloned() else { return };
        if self.preferred_editor == PreferredEditor::Ask {
            self.reveal_in_finder(&path, cx);
        } else {
            self.open_in_editor(&path, None, cx);
        }
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "changes_conflicts_tests.rs"]
mod changes_conflicts_tests;
#[cfg(test)]
#[path = "changes_conflicts_ui_tests.rs"]
mod changes_conflicts_ui_tests;
