//! The Changes panel's git ops — one action run off the UI thread through
//! `Workspace::run_git_op`. Split from `crate::changes` for the SLOC cap;
//! re-exported there so callers keep using `crate::changes::GitOp`.

/// One git action run off the UI thread.
pub(crate) enum GitOp {
    Stage(String),
    Unstage(String),
    /// `git apply --cached` with a single-hunk patch — stage one diff hunk.
    StageHunk {
        path: String,
        patch: String,
    },
    /// `git apply --cached --reverse` — unstage one diff hunk.
    UnstageHunk {
        path: String,
        patch: String,
    },
    /// Destructive per-file discard — the row's context menu confirms first.
    Discard(crate::git::FileChange),
    Commit(String),
    /// `commit --amend` — `None` keeps HEAD's message (`--no-edit`).
    CommitAmend(Option<String>),
    Push,
    CreatePr,
    Checkout(String),
    CreateBranch(String),
    RenameBranch {
        old: String,
        new: String,
    },
    DeleteBranch(String),
    Fetch,
    Pull,
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
            Self::StageHunk { path, patch } => crate::git::stage_hunk(dir, path, patch),
            Self::UnstageHunk { path, patch } => crate::git::unstage_hunk(dir, path, patch),
            Self::Discard(change) => crate::git::discard_file(dir, change),
            Self::Commit(message) => crate::git::commit(dir, message),
            Self::CommitAmend(message) => crate::git::commit_amend(dir, message.as_deref()),
            Self::Push => crate::git::push(dir),
            Self::CreatePr => crate::git::create_pr(dir, &[]),
            Self::Checkout(name) => crate::git::checkout(dir, name),
            Self::CreateBranch(name) => crate::git::create_branch(dir, name),
            Self::RenameBranch { old, new } => crate::git::rename_branch(dir, old, new),
            Self::DeleteBranch(name) => crate::git::delete_branch(dir, name),
            Self::Fetch => crate::git::fetch(dir),
            Self::Pull => crate::git::pull_ff(dir),
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
    /// stash succeeded, a cleared new-branch box when a branch was created,
    /// and a disarmed rename input when a rename landed (a failed rename
    /// stays armed so the typed name can be fixed and retried). Branch ops
    /// also re-list branches so an open picker shows the change.
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
                if matches!(op, GitOp::RenameBranch { .. }) {
                    self.git.rename_target = None;
                    self.git.rename_input.update(cx, |s, cx| s.set_value("", window, cx));
                }
                if matches!(op, GitOp::Stash(_)) {
                    self.git.stash_input.update(cx, |s, cx| s.set_value("", window, cx));
                }
                if matches!(op, GitOp::Checkout(_) | GitOp::CreateBranch(_) | GitOp::RenameBranch { .. } | GitOp::DeleteBranch(_)) {
                    self.refresh_branches(cx);
                }
                self.git.note = Some((text, false));
            },
            Err(e) => self.git.note = Some((e, true)),
        }
        cx.notify();
    }

    /// Put `path`'s unified diff on the clipboard — the row menu's "Copy
    /// Diff". A direct spawn rather than a `GitOp`: the payload is the diff
    /// itself (not a note), nothing mutates the index so `git.busy` doesn't
    /// gate it, and no refresh follows. `staged` picks the `--cached` half
    /// for a partially-staged file; untracked files get a synthesized
    /// new-file patch. Failures land as the panel's error note.
    pub fn copy_file_diff(&mut self, path: &str, staged: bool, cx: &mut Context<Self>) {
        let dir = self.project.root().to_path_buf();
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let job = path.clone();
            let result = cx.background_executor().spawn(async move { crate::git::file_diff(&dir, &job, staged) }).await;
            let _ = this.update(cx, |this, cx| this.land_copied_diff(&path, result, cx));
        })
        .detach();
    }

    /// Publish a fetched diff: the clipboard on success, the panel's note
    /// either way — a silent copy leaves the user guessing whether it worked.
    fn land_copied_diff(&mut self, path: &str, result: Result<String, String>, cx: &mut Context<Self>) {
        match result {
            Ok(diff) => {
                cx.write_to_clipboard(ClipboardItem::new_string(diff));
                self.git.note = Some((format!("Copied diff for {path}"), false));
            },
            Err(e) => self.git.note = Some((e, true)),
        }
        cx.notify();
    }

    /// `git fetch --prune` — the header's refresh button. The post-op
    /// `refresh_changes` picks up the new ahead/behind counts.
    pub fn fetch_remote(&mut self, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::Fetch, cx);
    }

    /// `git pull --ff-only` — the header's pull button. A diverged pull
    /// fails and its stderr lands as the note.
    pub fn pull_remote(&mut self, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::Pull, cx);
    }

    /// Discard one file's changes behind a native confirm — the row's
    /// context menu. Untracked files are deleted outright, so the prompt
    /// says so; tracked files revert to HEAD. The op re-lists the panel on
    /// landing, so the row disappears on its own.
    pub fn discard_change(&mut self, change: &crate::git::FileChange, window: &mut Window, cx: &mut Context<Self>) {
        let untracked = change.status == crate::git::ChangeStatus::Added && !change.staged;
        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Discard changes to “{}”?", change.path),
            Some(if untracked {
                "The file is untracked and will be deleted. This cannot be undone."
            } else {
                "This cannot be undone."
            }),
            &[PromptButton::ok("Discard"), PromptButton::cancel("Cancel")],
            cx,
        );
        let mut change = change.clone();
        change.diff = None;
        change.diff_load = 0;
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.run_git_op(GitOp::Discard(change), cx));
        })
        .detach();
    }

    /// Arm the header's rename input for `name` — prefilled with the current
    /// name and focused so typing replaces it. Choosing "Rename…" from a
    /// picker's row menu dismisses the popover, so the input lives on the
    /// always-visible branch header.
    pub fn begin_rename_branch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.git.rename_target = Some(name.to_string());
        self.git.rename_input.update(cx, |s, cx| {
            s.set_value(name.to_string(), window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Disarm the rename input without running the op — the header's ✕.
    pub fn cancel_rename_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.git.rename_target = None;
        self.git.rename_input.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    /// `git branch -m` the armed `rename_target` to the rename input's value
    /// — Enter in the input and the header's ✓ share this path. An empty or
    /// unchanged name is refused before spawning, same as the commit box.
    pub fn rename_branch(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.git.rename_target.clone() else { return };
        let new = self.git.rename_input.read(cx).value().trim().to_string();
        if new.is_empty() || new == old {
            return;
        }
        self.run_git_op(GitOp::RenameBranch { old, new }, cx);
    }

    /// `git branch -d <name>` behind a native confirm — the picker's row
    /// menu. The current branch is never offered this item; the guard stays
    /// so a stale menu can't delete the checked-out branch. Git's own
    /// not-merged refusal lands as the note — no force delete.
    pub fn delete_branch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.git.branch.as_ref().is_some_and(|b| b.name == name) {
            return;
        }
        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Delete branch “{name}”?"),
            Some("Only merged branches can be deleted."),
            &[PromptButton::ok("Delete"), PromptButton::cancel("Cancel")],
            cx,
        );
        let name = name.to_string();
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.run_git_op(GitOp::DeleteBranch(name), cx));
        })
        .detach();
    }

    /// Re-list local branches for the picker — runs when the picker opens so
    /// branches created outside the app show up. Off the UI thread like the
    /// rest of the panel's git calls.
    pub fn refresh_branches(&mut self, cx: &mut Context<Self>) {
        self.git.branches_generation += 1;
        let generation = self.git.branches_generation;
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let branches = cx.background_executor().spawn(async move { crate::git::list_branches(&dir) }).await;
            let _ = this.update(cx, |this, cx| this.land_branches(generation, branches, cx));
        })
        .detach();
    }

    /// Publish a fetched branch list — skipped when a newer fetch was
    /// requested while this one ran, same guard as `land_changes`.
    fn land_branches(&mut self, generation: u64, branches: Vec<crate::git::Branch>, cx: &mut Context<Self>) {
        if generation != self.git.branches_generation {
            return;
        }
        self.git.branches = branches;
        cx.notify();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "hunk_stage_tests.rs"]
mod hunk_stage_tests;
