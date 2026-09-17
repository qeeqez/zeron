//! File-inspect state and ops on `Workspace`: the open overlay's payload
//! (`FileInspect`), the background `git log --follow` / `git blame` fetches
//! the file menu's items kick off, the history row's per-file diff expand,
//! and the blame row's copy-sha click. Rendering lives in
//! `crate::views::file_inspect`; declared there via `#[path]` so `main.rs`
//! stays under the SLOC cap.

use gpui_kit::*;

use crate::git::{BlameLine, Commit, CommitDiff};
use crate::workspace::Workspace;

/// The open file-inspect overlay — which file, which view, and the fetched
/// payload (`None` while the background git call is in flight, `Some(Err)`
/// when it failed).
pub(crate) enum FileInspect {
    /// `git log --follow` rows for `path`.
    History { path: String, result: Option<Result<Vec<Commit>, String>> },
    /// `git blame --porcelain` rows for `path`.
    Blame { path: String, result: Option<Result<Vec<BlameLine>, String>> },
}

impl Workspace {
    /// The file menu's "File History" — open the overlay and fetch
    /// `git log --follow` for `rel` under `dir` on the background executor.
    /// `dir` is the changes scope's dir for worktree rows, the project root
    /// elsewhere.
    pub fn open_file_history_at(&mut self, dir: &std::path::Path, rel: &str, cx: &mut Context<Self>) {
        self.file_inspect = Some(FileInspect::History { path: rel.to_string(), result: None });
        let dir = dir.to_path_buf();
        let path = rel.to_string();
        cx.spawn(async move |this, cx| {
            let job = path.clone();
            let result = cx.background_executor().spawn(async move { crate::git::file_log(&dir, &job) }).await;
            let _ = this.update(cx, |this, cx| this.land_file_history(&path, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// The file menu's "Blame" — open the overlay and fetch
    /// `git blame --porcelain` for `rel` under `dir` on the background
    /// executor.
    pub fn open_file_blame_at(&mut self, dir: &std::path::Path, rel: &str, cx: &mut Context<Self>) {
        self.file_inspect = Some(FileInspect::Blame { path: rel.to_string(), result: None });
        let dir = dir.to_path_buf();
        let path = rel.to_string();
        cx.spawn(async move |this, cx| {
            let job = path.clone();
            let result = cx.background_executor().spawn(async move { crate::git::blame(&dir, &job) }).await;
            let _ = this.update(cx, |this, cx| this.land_file_blame(&path, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// Publish a fetched file history — dropped when the overlay moved on to
    /// another file/view or closed while the fetch ran. Failures land as the
    /// panel's error note too, so they're visible after the overlay closes.
    fn land_file_history(&mut self, path: &str, result: Result<Vec<Commit>, String>, cx: &mut Context<Self>) {
        if let Err(e) = &result {
            self.git.note = Some((e.clone(), true));
        }
        let Some(FileInspect::History { path: open, result: slot }) = &mut self.file_inspect else { return };
        if open != path {
            return;
        }
        *slot = Some(result);
        cx.notify();
    }

    /// Publish a fetched blame — same stale-drop and error-note rules as
    /// `land_file_history`.
    fn land_file_blame(&mut self, path: &str, result: Result<Vec<BlameLine>, String>, cx: &mut Context<Self>) {
        if let Err(e) = &result {
            self.git.note = Some((e.clone(), true));
        }
        let Some(FileInspect::Blame { path: open, result: slot }) = &mut self.file_inspect else { return };
        if open != path {
            return;
        }
        *slot = Some(result);
        cx.notify();
    }

    /// Expand/collapse a history row's diff for this file — same load-token
    /// dance as `toggle_commit_diff`, but the patch is `git show <sha> --
    /// <path>` so only this file's hunks render.
    pub fn toggle_history_diff(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(FileInspect::History { path, result: Some(Ok(commits)) }) = &mut self.file_inspect else { return };
        let Some(commit) = commits.get_mut(ix) else { return };
        if commit.diff.is_some() || commit.diff_load != 0 {
            commit.diff = None;
            commit.diff_load = 0;
            cx.notify();
            return;
        }
        let token = crate::changes::NEXT_DIFF_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        commit.diff_load = token;
        let sha = commit.hash.clone();
        let path = path.clone();
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let job = path.clone();
            let diff = cx.background_executor().spawn(async move { crate::git::commit_file_diff(&dir, &sha, &job) }).await;
            let _ = this.update(cx, |this, cx| this.land_history_diff(token, diff, cx));
        })
        .detach();
        cx.notify();
    }

    /// Store a loaded per-file commit diff on the history row stamped with
    /// `token` — dropped when the row collapsed or the overlay moved on.
    fn land_history_diff(&mut self, token: u64, diff: Result<CommitDiff, String>, cx: &mut Context<Self>) {
        if let Err(e) = &diff {
            self.git.note = Some((e.clone(), true));
        }
        let Some(FileInspect::History { result: Some(Ok(commits)), .. }) = &mut self.file_inspect else { return };
        let Some(row) = commits.iter_mut().find(|r| r.diff_load == token) else { return };
        row.diff_load = 0;
        row.diff = diff.ok();
        cx.notify();
    }

    /// Copy a blame row's commit hash — the row's click action.
    pub fn copy_blame_sha(&mut self, sha: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(sha.to_string()));
    }

    /// Close the overlay — the header ✕, the backdrop, and Esc share this.
    pub fn close_file_inspect(&mut self, cx: &mut Context<Self>) {
        self.file_inspect = None;
        cx.notify();
    }
}
