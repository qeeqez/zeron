//! The Changes panel's diff scope and the worktree diff-base picker: which
//! directory the file list comes from and which commit it diffs against,
//! the picker's ref list, and the per-row diff loads that honor the scope.
//! Split from `crate::changes` for the SLOC cap; re-exported there.

use std::path::PathBuf;

use gpui_kit::*;

use crate::changes::{DiffStamp, NEXT_DIFF_LOAD};
use crate::workspace::Workspace;

/// What the Changes panel's file list diffs: `dir` is the checkout the rows
/// live in and `base` — set only for worktree chats — is the merge-base
/// commit they diff against. `base: None` is the plain project mode:
/// `dir`'s working tree vs its own HEAD.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChangesScope {
    pub dir: PathBuf,
    pub base: Option<crate::worktree::diff::ResolvedBase>,
}

impl Workspace {
    /// The scope the panel's rows were collected under — row actions (diff
    /// loads, copy, discard, open) resolve paths and diffs against it. `dir`
    /// is re-derived from the active chat so a `project` swap can't leave
    /// rows pointing at the old root; `base` lands with each snapshot.
    pub(crate) fn changes_scope(&self) -> ChangesScope {
        let mut scope = self.changes_scope.clone();
        let chat = &self.chats[self.active];
        scope.dir = if chat.worktree {
            crate::worktree::workdir_for(chat, self.project.root())
        } else {
            self.project.root().to_path_buf()
        };
        scope
    }

    /// Expand/collapse a row's inline diff. Expanding stamps the row with a
    /// load token and fetches the diff on the background executor;
    /// collapsing drops the cached diff and clears the token so a
    /// still-running load is discarded when it lands.
    pub fn toggle_change_diff(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.changes.get(ix).is_some_and(|c| c.diff.is_some() || c.diff_load != 0) {
            let row = &mut self.changes[ix];
            row.diff = None;
            row.diff_load = 0;
            cx.notify();
            return;
        }
        self.start_diff_load(ix, cx);
    }

    /// Issue a background diff load for row `ix` under a fresh token — a
    /// still-running older load can't attach once this lands, so toggling
    /// `ignore_ws` mid-load can't be reverted by the stale result. Worktree
    /// rows diff against the scope's base commit instead of the index.
    fn start_diff_load(&mut self, ix: usize, cx: &mut Context<Self>) {
        let stamp: DiffStamp = (self.changes_generation, NEXT_DIFF_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let Some(row) = self.changes.get_mut(ix) else { return };
        row.diff_load = stamp.1;
        let change = row.clone();
        let scope = self.changes_scope();
        let ignore_ws = self.git.ignore_ws;
        cx.spawn(async move |this, cx| {
            let diff = cx
                .background_executor()
                .spawn(async move {
                    match scope.base_commit() {
                        Some(base) => crate::changes_diff::diff_for_file_at(&scope.dir, &change, Some(base), ignore_ws),
                        None => crate::changes_diff::diff_for_file(&scope.dir, &change, ignore_ws),
                    }
                })
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
        self.prune_review_comments(cx);
        cx.notify();
    }

    /// Re-list the picker's branches+tags — runs when the picker opens so
    /// refs created outside the app show up. Off the UI thread like the
    /// rest of the panel's git calls.
    pub fn refresh_diff_bases(&mut self, cx: &mut Context<Self>) {
        self.git.bases_generation += 1;
        let generation = self.git.bases_generation;
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let refs = cx.background_executor().spawn(async move { crate::git::list_diff_bases(&dir) }).await;
            let _ = this.update(cx, |this, cx| this.land_diff_bases(generation, refs, cx));
        })
        .detach();
    }

    /// Flip the ignore-whitespace diff filter, persist it, and re-issue the
    /// load for every expanded (or still-loading) row so the new flag takes
    /// effect without a collapse+re-expand.
    pub fn toggle_diff_ignore_ws(&mut self, cx: &mut Context<Self>) {
        self.git.ignore_ws = !self.git.ignore_ws;
        self.save_settings();
        for ix in 0..self.changes.len() {
            if self.changes[ix].diff.is_some() || self.changes[ix].diff_load != 0 {
                self.start_diff_load(ix, cx);
            }
        }
        cx.notify();
    }

    /// Publish a fetched ref list — skipped when a newer fetch was
    /// requested while this one ran, same guard as `land_branches`.
    fn land_diff_bases(&mut self, generation: u64, refs: Vec<crate::git::BaseRef>, cx: &mut Context<Self>) {
        if generation != self.git.bases_generation {
            return;
        }
        self.git.diff_bases = refs;
        cx.notify();
    }

    /// Pin the worktree chat's diff base to `picked` (`None` = back to the
    /// default), persist it on the chat, and recompute the panel.
    pub fn set_diff_base(&mut self, picked: Option<String>, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if chat.diff_base == picked {
            return;
        }
        chat.diff_base = picked;
        self.save();
        self.refresh_changes(cx);
    }
}

impl ChangesScope {
    /// The commit rows diff against, when the scope carries a base.
    pub(crate) fn base_commit(&self) -> Option<&str> {
        self.base.as_ref().map(|b| b.commit.as_str())
    }
}
