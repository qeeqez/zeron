//! The Changes panel's git ops — one action run off the UI thread through
//! `Workspace::run_git_op`. Split from `crate::changes` for the SLOC cap;
//! re-exported there so callers keep using `crate::changes::GitOp`.

/// One git action run off the UI thread.
pub(crate) enum GitOp {
    Stage(String),
    Unstage(String),
    Commit(String),
    /// `commit --amend` — `None` keeps HEAD's message (`--no-edit`).
    CommitAmend(Option<String>),
    Push,
    CreatePr,
    Checkout(String),
    CreateBranch(String),
    Revert(String),
    Stash(String),
    StashPop(String),
    StashApply(String),
    StashDrop(String),
    /// `checkout --ours|--theirs` + `add` for one conflicted path.
    Resolve(String, crate::changes_conflicts::ConflictSide),
}

impl GitOp {
    /// Run the op against `dir`; returns the op back with its outcome so the
    /// landing path can tell a commit (clears the message box) from the rest.
    pub(crate) fn run(self, dir: &std::path::Path) -> (Self, Result<String, String>) {
        let result = match &self {
            Self::Stage(path) => crate::git::stage(dir, path),
            Self::Unstage(path) => crate::git::unstage(dir, path),
            Self::Commit(message) => crate::git::commit(dir, message),
            Self::CommitAmend(message) => crate::git::commit_amend(dir, message.as_deref()),
            Self::Push => crate::git::push(dir),
            Self::CreatePr => crate::git::create_pr(dir, &[]),
            Self::Checkout(name) => crate::git::checkout(dir, name),
            Self::CreateBranch(name) => crate::git::create_branch(dir, name),
            Self::Revert(sha) => crate::git::revert(dir, sha),
            Self::Stash(message) => crate::git::stash_push(dir, message),
            Self::StashPop(name) => crate::git::stash_pop(dir, name),
            Self::StashApply(name) => crate::git::stash_apply(dir, name),
            Self::StashDrop(name) => crate::git::stash_drop(dir, name),
            Self::Resolve(path, side) => crate::changes_conflicts::resolve_conflict(dir, path, *side),
        };
        (self, result)
    }
}

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Run `op` on the background executor, then land its note and refresh
    /// the panel. Refused while another op is in flight — staging then
    /// committing mid-stage would race the index.
    pub(crate) fn run_git_op(&mut self, op: GitOp, cx: &mut Context<Self>) {
        if self.git.busy {
            return;
        }
        self.git.busy = true;
        self.git.note = None;
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let (op, result) = cx.background_executor().spawn(async move { op.run(&dir) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.land_git_op(op, result, window, cx);
                this.refresh_changes(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Publish an op's outcome: the note under the buttons, a cleared commit
    /// box and a reset amend toggle when a commit succeeded (a failed commit
    /// keeps the typed message so it isn't lost), a cleared stash box when a
    /// stash succeeded, and a cleared new-branch box when a branch was
    /// created. Branch ops also re-list branches so an open picker shows the
    /// switch.
    fn land_git_op(&mut self, op: GitOp, result: Result<String, String>, window: &mut Window, cx: &mut Context<Self>) {
        self.git.busy = false;
        match result {
            Ok(text) => {
                if matches!(op, GitOp::Commit(_) | GitOp::CommitAmend(_)) {
                    self.git.commit_input.update(cx, |s, cx| s.set_value("", window, cx));
                    self.git.amend = false;
                }
                if matches!(op, GitOp::CreateBranch(_)) {
                    self.git.new_branch_input.update(cx, |s, cx| s.set_value("", window, cx));
                }
                if matches!(op, GitOp::Stash(_)) {
                    self.git.stash_input.update(cx, |s, cx| s.set_value("", window, cx));
                }
                if matches!(op, GitOp::Checkout(_) | GitOp::CreateBranch(_)) {
                    self.refresh_branches(cx);
                }
                self.git.note = Some((text, false));
            },
            Err(e) => self.git.note = Some((e, true)),
        }
        cx.notify();
    }
}
