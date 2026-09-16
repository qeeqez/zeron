//! Stash actions on `Workspace`: the Stash button's `stash push -u` and the
//! per-row pop/apply/drop ops, all run through `run_git_op` so they can't
//! interleave with a stage/commit in flight and the panel refreshes (and
//! re-lists stashes) when they land. The list itself is collected by
//! `refresh_changes` in `crate::changes`; rendering lives in
//! `crate::views::changes_stash`.

use gpui_kit::*;

use crate::changes::GitOp;
use crate::workspace::Workspace;

/// Message `stash push -m` gets when the box is empty — a bare `git stash`
/// would record "WIP on <branch>", so this matches that convention.
const DEFAULT_STASH_MESSAGE: &str = "WIP";

impl Workspace {
    /// `git stash push -u -m <message>` from the stash box — Enter and the
    /// button share this path. An empty message falls back to "WIP"; the
    /// button stays enabled either way.
    pub fn stash_changes(&mut self, cx: &mut Context<Self>) {
        let message = self.git.stash_input.read(cx).value().trim().to_string();
        let message = if message.is_empty() { DEFAULT_STASH_MESSAGE.to_string() } else { message };
        self.run_git_op(GitOp::Stash(message), cx);
    }

    /// `git stash pop <name>` from a stash row's menu — applies the entry and
    /// drops it on success. A conflict keeps the entry and lands as the note.
    pub fn stash_pop(&mut self, name: &str, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::StashPop(name.to_string()), cx);
    }

    /// `git stash apply <name>` — restores the changes but keeps the entry.
    pub fn stash_apply(&mut self, name: &str, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::StashApply(name.to_string()), cx);
    }

    /// `git stash drop <name>` — removes the entry without applying it.
    pub fn stash_drop(&mut self, name: &str, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::StashDrop(name.to_string()), cx);
    }
}

#[cfg(test)]
#[path = "changes_stash_tests.rs"]
mod tests;
