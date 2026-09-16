//! Changes-panel state and git actions on `Workspace`: the branch header and
//! commit box (`ChangesGit`), the background collection that fills the file
//! list, per-file diff loads, and the stage/commit/push/create-PR ops that
//! shell out to `crate::git` in the project root. Rendering lives in
//! `crate::views::changes` and `crate::views::changes_git`.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::git::{Branch, BranchStatus, Commit, FileChange, StashEntry};
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
    pub branch: Option<BranchStatus>,
    pub commits: Vec<Commit>,
    pub stashes: Vec<StashEntry>,
    /// Unmerged paths from `git diff --diff-filter=U` — the conflicts
    /// section's rows; empty when no merge/rebase is mid-conflict.
    pub conflicts: Vec<String>,
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
    /// Recent commits for the "Recent commits" section — refreshed alongside
    /// `branch` by `refresh_changes`, empty on unborn HEADs.
    pub commits: Vec<Commit>,
    /// Stash entries for the "Stashes" section — refreshed alongside
    /// `commits` by `refresh_changes`, empty when nothing is stashed.
    pub stashes: Vec<StashEntry>,
    /// Conflicted (unmerged) paths — refreshed alongside `branch` by
    /// `refresh_changes`; drives the conflicts section's banner and rows.
    pub conflicts: Vec<String>,
    /// Bumped per `refresh_branches` request; a stale list can't overwrite a
    /// newer one when two fetches land out of order.
    branches_generation: u64,
    /// Commit message input — Enter commits, same as the button.
    pub commit_input: Entity<InputState>,
    /// Stash message input — Enter stashes, same as the button; an empty
    /// message falls back to "WIP".
    pub stash_input: Entity<InputState>,
    /// New-branch name input in the picker — Enter creates and switches.
    pub new_branch_input: Entity<InputState>,
    /// A git op is running on the background executor — buttons stay up but
    /// re-entry is refused so ops can't interleave.
    pub busy: bool,
    /// An AI commit-message turn is in flight (see `crate::changes_generate`)
    /// — the ✦ button shows a spinner and refuses re-entry.
    pub generating: bool,
    /// Last op's outcome — `(text, is_error)`; `None` before the first op.
    pub note: Option<(String, bool)>,
}

impl ChangesGit {
    /// Build the state and wire Enter in the commit input to `commit_staged`
    /// and Enter in the new-branch input to `create_branch`.
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
            commits: Vec::new(),
            stashes: Vec::new(),
            conflicts: Vec::new(),
            branches_generation: 0,
            commit_input,
            stash_input,
            new_branch_input,
            busy: false,
            generating: false,
            note: None,
        }
    }
}

/// Git ops dispatched off the UI thread — split into `changes_ops.rs` for
/// the SLOC cap; re-exported so callers keep using `crate::changes::GitOp`.
#[path = "changes_ops.rs"]
pub(crate) mod ops;
pub(crate) use ops::GitOp;

impl Workspace {
    /// Expand/collapse a row's inline diff. Expanding stamps the row with a
    /// load token and fetches the working-tree diff on the background
    /// executor; collapsing drops the cached diff and clears the token so a
    /// still-running load is discarded when it lands.
    pub fn toggle_change_diff(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.changes.get(ix).is_some_and(|c| c.diff.is_some() || c.diff_load != 0) {
            let row = &mut self.changes[ix];
            row.diff = None;
            row.diff_load = 0;
            cx.notify();
            return;
        }
        let stamp: DiffStamp = (self.changes_generation, NEXT_DIFF_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let row = &mut self.changes[ix];
        row.diff_load = stamp.1;
        let change = row.clone();
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let diff = cx
                .background_executor()
                .spawn(async move { crate::changes_diff::diff_for_file(&dir, &change) })
                .await;
            let _ = this.update(cx, |this, cx| this.land_change_diff(stamp, diff, cx));
        })
        .detach();
    }

    /// Store a loaded diff on the row stamped with the stamp's token —
    /// skipped when the list generation moved on (a refresh landed or is in
    /// flight) or no row still waits on that token (collapsed or re-expanded
    /// under the load). The token is unique per load, so it identifies the row.
    pub(crate) fn land_change_diff(&mut self, stamp: DiffStamp, diff: Option<crate::changes_diff::FileDiff>, cx: &mut Context<Self>) {
        if stamp.0 != self.changes_generation {
            return;
        }
        let Some(row) = self.changes.iter_mut().find(|r| r.diff_load == stamp.1) else { return };
        row.diff_load = 0;
        row.diff = diff;
        cx.notify();
    }

    /// Re-run git collection for the Changes panel: the file list plus the
    /// branch header and recent commits. Collection shells out to several git
    /// processes and reads untracked files, so it runs on the background
    /// executor and publishes the result back when done.
    pub fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        self.changes_generation += 1;
        let generation = self.changes_generation;
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move {
                    ChangesSnapshot {
                        changes: crate::git::collect(&root),
                        branch: crate::git::branch_status(&root),
                        commits: crate::git::log(&root, 20),
                        stashes: crate::git::stash_list(&root),
                        conflicts: crate::changes_conflicts::conflicted_files(&root),
                    }
                })
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
        self.git.branch = snapshot.branch;
        self.git.commits = snapshot.commits;
        self.git.stashes = snapshot.stashes;
        self.git.conflicts = snapshot.conflicts;
        cx.notify();
    }

    /// Stage or unstage the file at row `ix` — `git add` / `git restore
    /// --staged` — then refresh so the row's staged marker and counts update.
    pub fn toggle_change_stage(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get(ix) else { return };
        let op = if change.staged { GitOp::Unstage(change.path.clone()) } else { GitOp::Stage(change.path.clone()) };
        self.run_git_op(op, cx);
    }

    /// Commit the staged files with the commit box's message. An empty
    /// message is refused before spawning — the button is disabled in the
    /// same case, so this only guards Enter and tests.
    pub fn commit_staged(&mut self, cx: &mut Context<Self>) {
        let message = self.git.commit_input.read(cx).value().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.run_git_op(GitOp::Commit(message), cx);
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
    fn land_branches(&mut self, generation: u64, branches: Vec<Branch>, cx: &mut Context<Self>) {
        if generation != self.git.branches_generation {
            return;
        }
        self.git.branches = branches;
        cx.notify();
    }

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
    /// box when a commit succeeded (a failed commit keeps the typed message
    /// so it isn't lost), a cleared stash box when a stash succeeded, and a
    /// cleared new-branch box when a branch was created. Branch ops also
    /// re-list branches so an open picker shows the switch.
    fn land_git_op(&mut self, op: GitOp, result: Result<String, String>, window: &mut Window, cx: &mut Context<Self>) {
        self.git.busy = false;
        match result {
            Ok(text) => {
                if matches!(op, GitOp::Commit(_)) {
                    self.git.commit_input.update(cx, |s, cx| s.set_value("", window, cx));
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

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "changes_stale_tests.rs"]
mod changes_stale_tests;
#[cfg(test)]
#[path = "changes_tests.rs"]
mod changes_tests;
