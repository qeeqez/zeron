//! Changes-panel state and git actions on `Workspace`: the branch header and
//! commit box (`ChangesGit`), the background collection that fills the file
//! list, per-file diff loads, and the stage/commit/push/create-PR ops that
//! shell out to `crate::git` in the project root. Rendering lives in
//! `crate::views::changes` and `crate::views::changes_git`.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::git::{BranchStatus, FileChange};
use crate::workspace::Workspace;

/// Token source for in-flight row-diff loads — each expand stamps the row
/// with a fresh id so a stale result can't attach after collapse+re-expand.
static NEXT_DIFF_LOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What a background diff load was issued under: `(change-list generation,
/// row load token)`. Both must still match when the result lands — a refresh
/// bumps the generation, collapse/re-expand changes the token — or the diff
/// is stale and gets discarded.
type DiffStamp = (u64, u64);

/// Git-action state for the Changes panel: the branch header, the commit
/// message input, a busy flag that serializes ops, and the status note shown
/// under the buttons.
pub struct ChangesGit {
    /// Current branch + ahead/behind — `None` when the project isn't a git
    /// repo, which hides the whole action block.
    pub branch: Option<BranchStatus>,
    /// Commit message input — Enter commits, same as the button.
    pub commit_input: Entity<InputState>,
    /// A git op is running on the background executor — buttons stay up but
    /// re-entry is refused so ops can't interleave.
    pub busy: bool,
    /// Last op's outcome — `(text, is_error)`; `None` before the first op.
    pub note: Option<(String, bool)>,
}

impl ChangesGit {
    /// Build the state and wire Enter in the commit input to `commit_staged`.
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message…"));
        cx.subscribe(&commit_input, |this: &mut Workspace, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_staged(cx);
            }
        })
        .detach();
        Self { branch: None, commit_input, busy: false, note: None }
    }
}

/// One git action run off the UI thread.
enum GitOp {
    Stage(String),
    Unstage(String),
    Commit(String),
    Push,
    CreatePr,
}

impl GitOp {
    /// Run the op against `dir`; returns the op back with its outcome so the
    /// landing path can tell a commit (clears the message box) from the rest.
    fn run(self, dir: &std::path::Path) -> (Self, Result<String, String>) {
        let result = match &self {
            Self::Stage(path) => crate::git::stage(dir, path),
            Self::Unstage(path) => crate::git::unstage(dir, path),
            Self::Commit(message) => crate::git::commit(dir, message),
            Self::Push => crate::git::push(dir),
            Self::CreatePr => crate::git::create_pr(dir, &[]),
        };
        (self, result)
    }
}

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
    /// branch header. Collection shells out to several git processes and
    /// reads untracked files, so it runs on the background executor and
    /// publishes the result back when done.
    pub fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        self.changes_generation += 1;
        let generation = self.changes_generation;
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let (changes, branch) = cx
                .background_executor()
                .spawn(async move { (crate::git::collect(&root), crate::git::branch_status(&root)) })
                .await;
            let _ = this.update(cx, |this, cx| this.land_changes(generation, changes, branch, cx));
        })
        .detach();
    }

    /// Publish a collected change list — skipped when a newer refresh was
    /// requested while this one ran, so an older result can't revert the
    /// panel to a stale snapshot.
    pub(crate) fn land_changes(&mut self, generation: u64, changes: Vec<FileChange>, branch: Option<BranchStatus>, cx: &mut Context<Self>) {
        if generation != self.changes_generation {
            return;
        }
        self.changes = changes;
        self.git.branch = branch;
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

    /// Run `op` on the background executor, then land its note and refresh
    /// the panel. Refused while another op is in flight — staging then
    /// committing mid-stage would race the index.
    fn run_git_op(&mut self, op: GitOp, cx: &mut Context<Self>) {
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

    /// Publish an op's outcome: the note under the buttons, and a cleared
    /// commit box when a commit succeeded (a failed commit keeps the typed
    /// message so it isn't lost).
    fn land_git_op(&mut self, op: GitOp, result: Result<String, String>, window: &mut Window, cx: &mut Context<Self>) {
        self.git.busy = false;
        match result {
            Ok(text) => {
                if matches!(op, GitOp::Commit(_)) {
                    self.git.commit_input.update(cx, |s, cx| s.set_value("", window, cx));
                }
                self.git.note = Some((text, false));
            },
            Err(e) => self.git.note = Some((e, true)),
        }
        cx.notify();
    }
}
