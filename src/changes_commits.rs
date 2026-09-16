//! Recent-commits state on `Workspace`: expanding a "Recent commits" row to
//! load its `git show` patch on the background executor, the revert op the
//! row's context menu runs, and the PR row's standalone refresh. The list
//! itself is collected by `refresh_changes` in `crate::changes`; rendering
//! lives in `crate::views::changes_commits` and `crate::views::changes_pr`.

use gpui_kit::*;

use crate::changes::{DiffStamp, GitOp, NEXT_DIFF_LOAD};
use crate::workspace::Workspace;

impl Workspace {
    /// Expand/collapse a commit row's inline diff — same load-token dance as
    /// `toggle_change_diff`: expanding stamps the row and fetches `git show`
    /// off the UI thread; collapsing drops the cached diff so a still-running
    /// load is discarded when it lands.
    pub fn toggle_commit_diff(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.git.commits.get(ix).is_some_and(|c| c.diff.is_some() || c.diff_load != 0) {
            let row = &mut self.git.commits[ix];
            row.diff = None;
            row.diff_load = 0;
            cx.notify();
            return;
        }
        let stamp: DiffStamp = (self.changes_generation, NEXT_DIFF_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let row = &mut self.git.commits[ix];
        row.diff_load = stamp.1;
        let sha = row.hash.clone();
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let diff = cx.background_executor().spawn(async move { crate::git::commit_diff(&dir, &sha) }).await;
            let _ = this.update(cx, |this, cx| this.land_commit_diff(stamp, diff, cx));
        })
        .detach();
    }

    /// Store a loaded commit diff on the row stamped with the stamp's token —
    /// skipped when the list generation moved on (a refresh landed or is in
    /// flight) or no row still waits on that token. Same guard as
    /// `land_change_diff`.
    pub(crate) fn land_commit_diff(&mut self, stamp: DiffStamp, diff: Option<crate::git::CommitDiff>, cx: &mut Context<Self>) {
        if stamp.0 != self.changes_generation {
            return;
        }
        let Some(row) = self.git.commits.iter_mut().find(|r| r.diff_load == stamp.1) else { return };
        row.diff_load = 0;
        row.diff = diff;
        cx.notify();
    }

    /// `git revert --no-edit <sha>` from a commit row's context menu — a new
    /// commit undoing `sha`, never a history rewrite. Runs through
    /// `run_git_op` so it can't interleave with a stage/commit in flight and
    /// the panel refreshes (and re-lists commits) when it lands.
    pub fn revert_commit(&mut self, sha: &str, cx: &mut Context<Self>) {
        self.run_git_op(GitOp::Revert(sha.to_string()), cx);
    }

    /// Re-fetch the current branch's PR status — the PR row's refresh
    /// button. One `gh pr view` on the background executor; the result is
    /// stamped with the changes generation so a full refresh requested
    /// while it runs (which fetches its own PR status) discards it.
    pub fn refresh_pr(&mut self, cx: &mut Context<Self>) {
        let generation = self.changes_generation;
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let pr = cx.background_executor().spawn(async move { crate::git::pr_status(&dir, &[]) }).await;
            let _ = this.update(cx, |this, cx| this.land_pr(generation, pr, cx));
        })
        .detach();
    }

    /// Publish a fetched PR status — skipped when a `refresh_changes` was
    /// requested while the fetch ran, since that refresh's snapshot carries
    /// its own (newer) PR status. Same guard as `land_commit_diff`.
    pub(crate) fn land_pr(&mut self, generation: u64, pr: Option<crate::git::PrStatus>, cx: &mut Context<Self>) {
        if generation != self.changes_generation {
            return;
        }
        self.git.pr = pr;
        cx.notify();
    }
}
