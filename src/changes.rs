//! Changes-panel state and git actions on `Workspace`: the branch header and
//! commit box (`ChangesGit`), the background collection that fills the file
//! list, per-file diff loads, and the stage/commit/push/create-PR ops that
//! shell out to `crate::git` in the project root. Rendering lives in
//! `crate::views::changes` and `crate::views::changes_git`.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::git::{Branch, BranchStatus, Commit, FileChange, PrStatus, StashEntry};
use crate::workspace::Workspace;

/// Token source for in-flight row-diff loads — each expand stamps the row
/// with a fresh id so a stale result can't attach after collapse+re-expand.
pub(crate) static NEXT_DIFF_LOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What a background diff load was issued under: `(change-list generation,
/// row load token)`. Both must still match when the result lands — a refresh
/// bumps the generation, collapse/re-expand changes the token — or the diff
/// is stale and gets discarded.
pub(crate) type DiffStamp = (u64, u64);

/// One collected snapshot of the panel's git state — the file list, branch
/// header, and recent commits land together so a stale refresh can't publish
/// a half-new snapshot.
pub(crate) struct ChangesSnapshot {
    pub changes: Vec<FileChange>,
    /// The scope `changes` was collected under — lands on the workspace so
    /// row actions diff and resolve paths against the same base.
    pub scope: crate::changes::base::ChangesScope,
    pub branch: Option<BranchStatus>,
    pub commits: Vec<Commit>,
    pub stashes: Vec<StashEntry>,
    /// Unmerged paths from `git diff --diff-filter=U` — the conflicts
    /// section's rows; empty when no merge/rebase is mid-conflict.
    pub conflicts: Vec<String>,
    /// The current branch's PR — `None` without `gh`, outside a repo, or
    /// when the branch has no PR; the row hides in every case.
    pub pr: Option<PrStatus>,
}

/// Git-action state for the Changes panel: the branch header, the commit
/// message input, a busy flag that serializes ops, and the status note shown
/// under the buttons.
pub struct ChangesGit {
    /// Current branch + ahead/behind — `None` when the project isn't a git
    /// repo, which hides the whole action block.
    pub branch: Option<BranchStatus>,
    /// Local branches for the picker's list — filled when the picker opens,
    /// empty until then.
    pub branches: Vec<Branch>,
    /// Branches + tags for the worktree diff-base picker — filled when the
    /// picker opens, empty until then.
    pub diff_bases: Vec<crate::git::BaseRef>,
    /// Recent commits for the "Recent commits" section — refreshed alongside
    /// `branch` by `refresh_changes`, empty on unborn HEADs.
    pub commits: Vec<Commit>,
    /// Stash entries for the "Stashes" section — refreshed alongside
    /// `commits` by `refresh_changes`, empty when nothing is stashed.
    pub stashes: Vec<StashEntry>,
    /// Conflicted (unmerged) paths — refreshed alongside `branch` by
    /// `refresh_changes`; drives the conflicts section's banner and rows.
    pub conflicts: Vec<String>,
    /// The current branch's PR — refreshed alongside `branch` by
    /// `refresh_changes` and by the row's own refresh button. `None` hides
    /// the row (no `gh`, not a repo, or no PR for the branch).
    pub pr: Option<PrStatus>,
    /// Bumped per `refresh_branches` request; a stale list can't overwrite a
    /// newer one when two fetches land out of order.
    branches_generation: u64,
    /// Bumped per `refresh_diff_bases` request — same stale-landing guard
    /// as `branches_generation`.
    bases_generation: u64,
    /// Commit message input — Enter commits, same as the button.
    pub commit_input: Entity<InputState>,
    /// Stash message input — Enter stashes, same as the button; an empty
    /// message falls back to "WIP".
    pub stash_input: Entity<InputState>,
    /// New-branch name input in the picker — Enter creates and switches.
    pub new_branch_input: Entity<InputState>,
    /// Rename input in the branch header — shown while `rename_target` is
    /// set; Enter runs the rename. Kept off the picker because choosing
    /// "Rename…" from a row's menu dismisses the popover.
    pub rename_input: Entity<InputState>,
    /// Branch being renamed — `Some` while the header's rename input is up.
    /// Stays armed when a rename fails so the typed name can be fixed and
    /// retried; cleared on success and by the input's cancel button.
    pub rename_target: Option<String>,
    /// A git op is running on the background executor — buttons stay up but
    /// re-entry is refused so ops can't interleave.
    pub busy: bool,
    /// An AI commit-message turn is in flight (see `crate::changes_generate`)
    /// — the ✦ button shows a spinner and refuses re-entry.
    pub generating: bool,
    /// Amend mode for the commit box — the button reads "Amend", an empty
    /// message is allowed (`--no-edit`), and a successful amend resets it.
    /// Runtime only, never persisted.
    pub amend: bool,
    /// Last op's outcome — `(text, is_error)`; `None` before the first op.
    pub note: Option<(String, bool)>,
    /// Expanded diffs hide whitespace-only changes — the panel header's
    /// space toggle; persisted as `Settings.diff_ignore_ws` and passed to
    /// `git diff` as `--ignore-all-space`.
    pub ignore_ws: bool,
}

impl ChangesGit {
    /// Build the state and wire Enter in the commit input to `commit_staged`,
    /// Enter in the new-branch input to `create_branch`, and Enter in the
    /// rename input to `rename_branch`.
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message…"));
        cx.subscribe(&commit_input, |this: &mut Workspace, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_staged(cx);
            }
        })
        .detach();
        let new_branch_input = cx.new(|cx| InputState::new(window, cx).placeholder("New branch name…"));
        cx.subscribe(&new_branch_input, |this: &mut Workspace, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create_branch(cx);
            }
        })
        .detach();
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Rename branch to…"));
        cx.subscribe(&rename_input, |this: &mut Workspace, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.rename_branch(cx);
            }
        })
        .detach();
        let stash_input = cx.new(|cx| InputState::new(window, cx).placeholder("Stash message (optional)…"));
        cx.subscribe(&stash_input, |this: &mut Workspace, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.stash_changes(cx);
            }
        })
        .detach();
        Self {
            branch: None,
            branches: Vec::new(),
            diff_bases: Vec::new(),
            commits: Vec::new(),
            stashes: Vec::new(),
            conflicts: Vec::new(),
            pr: None,
            branches_generation: 0,
            bases_generation: 0,
            commit_input,
            stash_input,
            new_branch_input,
            rename_input,
            rename_target: None,
            busy: false,
            generating: false,
            amend: false,
            note: None,
            ignore_ws: false,
        }
    }
}

/// Git ops dispatched off the UI thread — split into `changes_ops.rs` for
/// the SLOC cap; re-exported so callers keep using `crate::changes::GitOp`.
#[path = "changes_ops.rs"]
pub(crate) mod ops;
pub(crate) use ops::GitOp;

/// The panel's diff scope, the worktree diff-base picker ops, and per-row
/// diff loads — split into `changes_base.rs` for the SLOC cap; re-exported
/// so callers keep using `crate::changes::ChangesScope`.
#[path = "changes_base.rs"]
pub(crate) mod base;
pub(crate) use base::ChangesScope;

/// Branch header ops — split into `changes_branches.rs` for the SLOC cap.
#[path = "changes_branches.rs"]
pub(crate) mod branches;

impl Workspace {
    /// Re-run git collection for the Changes panel: the file list plus the
    /// branch header and recent commits. A worktree chat's list diffs its
    /// checkout against the resolved diff base (`crate::worktree::diff`);
    /// anything else shows `dir`'s working tree vs its own HEAD. Collection
    /// shells out to several git processes and reads untracked files, so it
    /// runs on the background executor and publishes the result back.
    pub fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        self.changes_generation += 1;
        let generation = self.changes_generation;
        let root = self.project.root().to_path_buf();
        let chat = &self.chats[self.active];
        let worktree = chat.worktree;
        let dir = if worktree { crate::worktree::workdir_for(chat, &root) } else { root.clone() };
        let picked = chat.diff_base.clone();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { collect_snapshot(&root, &dir, worktree, picked.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| this.land_changes(generation, snapshot, cx));
        })
        .detach();
    }

    /// Publish a collected snapshot — skipped when a newer refresh was
    /// requested while this one ran, so an older result can't revert the
    /// panel to a stale snapshot.
    pub(crate) fn land_changes(&mut self, generation: u64, snapshot: ChangesSnapshot, cx: &mut Context<Self>) {
        if generation != self.changes_generation {
            return;
        }
        self.changes = snapshot.changes;
        self.changes_scope = snapshot.scope;
        self.git.branch = snapshot.branch;
        self.git.commits = snapshot.commits;
        self.git.stashes = snapshot.stashes;
        self.git.conflicts = snapshot.conflicts;
        self.git.pr = snapshot.pr;
        self.prune_review_comments(cx);
        cx.notify();
    }

    /// Commit the staged files with the commit box's message — or amend HEAD
    /// when the amend toggle is on. An empty message is refused before
    /// spawning unless amending (`--no-edit` keeps HEAD's message); the
    /// button is disabled in the same case, so this only guards Enter and
    /// tests.
    pub fn commit_staged(&mut self, cx: &mut Context<Self>) {
        let message = self.git.commit_input.read(cx).value().trim().to_string();
        if self.git.amend {
            let message = (!message.is_empty()).then_some(message);
            self.run_git_op(GitOp::CommitAmend(message), cx);
            return;
        }
        if message.is_empty() {
            return;
        }
        self.run_git_op(GitOp::Commit(message), cx);
    }

    /// Flip the commit box's amend mode. Turning it on prefills the box with
    /// HEAD's subject when it's empty (fetched off the UI thread like the
    /// panel's other git calls); a typed message is kept as the replacement.
    pub fn toggle_commit_amend(&mut self, cx: &mut Context<Self>) {
        self.git.amend = !self.git.amend;
        let prefill = self.git.amend && self.git.commit_input.read(cx).value().trim().is_empty();
        if !prefill {
            cx.notify();
            return;
        }
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let subject = cx.background_executor().spawn(async move { crate::git::last_commit_subject(&dir) }).await;
            let _ = this.update_in(cx, |this, window, cx| this.land_amend_prefill(subject, window, cx));
        })
        .detach();
        cx.notify();
    }

    /// Fill the commit box with HEAD's subject after amend mode turned on —
    /// skipped when the user toggled amend back off while the fetch ran, or
    /// when HEAD has no subject to offer (unborn, non-repo).
    fn land_amend_prefill(&mut self, subject: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.git.amend {
            return;
        }
        if let Some(subject) = subject.filter(|s| !s.is_empty()) {
            self.git.commit_input.update(cx, |s, cx| s.set_value(subject, window, cx));
        }
    }

    /// `git push` the current branch (setting `-u origin HEAD` when it has no
    /// upstream).
    pub fn push_changes(&mut self, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::Push, cx);
    }

    /// Push, then `gh pr create --fill`; without `gh` the push still lands
    /// and the note says to open the PR by hand.
    pub fn create_pr(&mut self, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::CreatePr, cx);
    }

    /// `git checkout <name>` in the project root — the picker's branch rows.
    /// Picking the current branch is a no-op; a dirty tree that would lose
    /// edits makes git refuse and its stderr lands as the note.
    pub fn checkout_branch(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.git.branch.as_ref().is_some_and(|b| b.name == name) {
            return;
        }
        self.run_git_op(GitOp::Checkout(name.to_string()), cx);
    }

    /// `git checkout -b <name>` from the picker's new-branch input — Enter
    /// and the button share this path. An empty name is refused before
    /// spawning, same as the commit box.
    pub fn create_branch(&mut self, cx: &mut Context<Self>) {
        let name = self.git.new_branch_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        self.run_git_op(GitOp::CreateBranch(name), cx);
    }
}

/// One `refresh_changes` collection, run on the background executor: the
/// file list under `dir` (a worktree chat's rows diff against the resolved
/// base via `crate::worktree::diff`), plus the project-root git state the
/// header and action block read.
fn collect_snapshot(root: &std::path::Path, dir: &std::path::Path, worktree: bool, picked: Option<&str>) -> ChangesSnapshot {
    let base = worktree.then(|| crate::worktree::diff::resolve_diff_base(root, dir, picked)).flatten();
    let changes = match &base {
        Some(b) => crate::worktree::diff::worktree_changes(dir, &b.commit).unwrap_or_else(|_| crate::git::collect(dir)),
        None => crate::git::collect(dir),
    };
    let branch = crate::git::branch_status(root);
    ChangesSnapshot {
        changes,
        scope: crate::changes::base::ChangesScope { dir: dir.to_path_buf(), base },
        // `gh pr view` only makes sense in a repo — skip the spawn entirely
        // when there's no branch.
        pr: branch.as_ref().and_then(|_| crate::git::pr_status(root, &[])),
        branch,
        commits: crate::git::log(root, 20),
        stashes: crate::git::stash_list(root),
        conflicts: crate::changes_conflicts::conflicted_files(root),
    }
}
// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "changes_amend_tests.rs"]
mod changes_amend_tests;
#[cfg(test)]
#[path = "changes_branch_tests.rs"]
mod changes_branch_tests;
#[cfg(test)]
#[path = "changes_copy_tests.rs"]
mod changes_copy_tests;
#[cfg(test)]
#[path = "changes_diff_base_tests.rs"]
mod changes_diff_base_tests;
#[cfg(test)]
#[path = "changes_discard_tests.rs"]
mod changes_discard_tests;
#[cfg(test)]
#[path = "changes_stale_tests.rs"]
mod changes_stale_tests;
#[cfg(test)]
#[path = "changes_tests.rs"]
mod changes_tests;
#[cfg(test)]
#[path = "diff_ignore_ws_tests.rs"]
mod diff_ignore_ws_tests;
// trivial
