//! The Changes panel's git ops — one action run off the UI thread through
//! `Workspace::run_git_op`. Split from `crate::changes` for the SLOC cap;
//! re-exported there so callers keep using `crate::changes::GitOp`.

/// One git action run off the UI thread.
pub(crate) enum GitOp {
    Stage(String),
    Unstage(String),
    Commit(String),
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
