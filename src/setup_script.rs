//! Per-project setup script — the desktop counterpart of Codex cloud's
//! environment setup. When `worktree::create` adds a thread worktree, the
//! project's `setup_script` (stored in `state.json`, edited under Settings →
//! Project) runs as `sh -c <script>` inside the new checkout — install deps,
//! symlink env files, …
//!
//! The script runs on a spawned thread so a slow `npm install` never blocks
//! the UI. Its outcome lands in the thread's transcript as a note plus an
//! activity-feed entry: `apply_thread_defaults` claims the run's
//! `JoinHandle` from `PENDING` and awaits it on the background executor;
//! runs nobody claims (forked threads, headless callers) are drained by
//! `Workspace::save` once their thread finishes. A failure is only ever a
//! note — it never blocks or fails the chat.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::thread::JoinHandle;
use std::time::SystemTime;

use gpui_kit::*;
use parking_lot::Mutex;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// The result of one setup-script run — what the note and feed entry say.
pub(crate) struct Outcome {
    /// The worktree the script ran in — links the outcome to its chat.
    pub dir: PathBuf,
    /// `Ok(output tail)` on exit 0, `Err(detail)` on a nonzero exit or a
    /// spawn failure.
    pub result: Result<String, String>,
}

/// Runs nobody has claimed yet — `take_pending` hands the handle to the
/// workspace that owns the chat, `take_outcomes` reaps finished leftovers.
type Pending = Vec<(PathBuf, JoinHandle<Outcome>)>;
static PENDING: LazyLock<Mutex<Pending>> = LazyLock::new(|| Mutex::new(Vec::new()));

/// Run the project's setup script inside `dir` on a spawned thread.
/// Returns `false` (and spawns nothing) when no script is configured —
/// the common case stays a no-op.
pub(crate) fn spawn(project: &crate::project::Project, dir: &Path) -> bool {
    let script = project.load_state().setup_script;
    if script.trim().is_empty() {
        return false;
    }
    let workdir = dir.to_path_buf();
    let out_dir = workdir.clone();
    PENDING.lock().push((workdir, std::thread::spawn(move || run(&out_dir, &script))));
    true
}

/// Claim the pending run for `dir`, if one exists — the caller awaits the
/// handle and lands the note itself.
pub(crate) fn take_pending(dir: &Path) -> Option<JoinHandle<Outcome>> {
    let mut pending = PENDING.lock();
    let ix = pending.iter().position(|(d, _)| d == dir)?;
    Some(pending.remove(ix).1)
}

/// Finished runs still in `PENDING` — drained by `Workspace::save` so
/// unclaimed worktrees (forks) still get their note on the next save.
pub(crate) fn take_outcomes() -> Vec<Outcome> {
    let mut pending = PENDING.lock();
    let mut done = Vec::new();
    let mut ix = 0;
    while ix < pending.len() {
        if pending[ix].1.is_finished() {
            let (dir, handle) = pending.remove(ix);
            done.push(join(dir, handle));
        } else {
            ix += 1;
        }
    }
    done
}

/// Join a finished run; a panicked script thread becomes a failed outcome
/// rather than propagating.
fn join(dir: PathBuf, handle: JoinHandle<Outcome>) -> Outcome {
    handle
        .join()
        .unwrap_or_else(|_| Outcome { dir, result: Err("setup script thread panicked".into()) })
}

/// `sh -c <script>` in `dir`, output tail on success, status + output on
/// failure. Never panics — every failure mode is an `Err` string.
fn run(dir: &Path, script: &str) -> Outcome {
    let result = std::process::Command::new("sh")
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("couldn't run `sh -c`: {e}"))
        .and_then(|out| {
            let text = tail(&format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
            if out.status.success() { Ok(text) } else { Err(format!("exited with {}\n{text}", out.status)) }
        });
    Outcome { dir: dir.to_path_buf(), result }
}

/// The last `MAX_LINES` of script output — enough to see why a setup
/// failed without flooding the transcript.
fn tail(output: &str) -> String {
    const MAX_LINES: usize = 40;
    let trimmed = output.trim();
    let skip = trimmed.lines().count().saturating_sub(MAX_LINES);
    trimmed.lines().skip(skip).collect::<Vec<_>>().join("\n")
}

/// The transcript note for a finished run.
fn note_text(outcome: &Outcome) -> String {
    match &outcome.result {
        Ok(out) if out.is_empty() => "**Setup script finished.**".to_string(),
        Ok(out) => format!("**Setup script finished.**\n\n```\n{out}\n```"),
        Err(e) => format!("**Setup script failed** — the thread keeps its worktree.\n\n```\n{e}\n```"),
    }
}

impl Workspace {
    /// Await the setup run for `dir` (claimed from `PENDING`) on the
    /// background executor, then land its note. No-op when nothing ran.
    pub(crate) fn watch_setup_script(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let Some(handle) = take_pending(&dir) else { return };
        cx.spawn(async move |this, cx| {
            let outcome = cx.background_executor().spawn(async move { join(dir, handle) }).await;
            let _ = this.update(cx, |this, cx| this.land_setup_outcome(outcome, cx));
        })
        .detach();
    }

    /// Land a finished run: a note on the owning chat plus an activity-feed
    /// entry, then persist.
    fn land_setup_outcome(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        let active = self.note_setup_outcome(outcome);
        if active && self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        cx.notify();
        self.save();
    }

    /// Append the outcome's note to the chat that owns `dir` and record it
    /// in the activity feed. Returns whether that chat is the active one —
    /// callers with a `Context` use it for the scroller append.
    pub(crate) fn note_setup_outcome(&mut self, outcome: Outcome) -> bool {
        let text = note_text(&outcome);
        let kind = if outcome.result.is_ok() {
            crate::activity::ActivityKind::TurnFinished
        } else {
            crate::activity::ActivityKind::Error
        };
        let workdir = outcome.dir.to_string_lossy();
        let Some(ix) = self.chats.iter().position(|c| c.workdir == workdir) else {
            // The chat is gone (deleted mid-run) — keep the outcome in the
            // feed so the failure isn't lost.
            let title: SharedString = outcome.dir.file_name().unwrap_or_default().to_string_lossy().into_owned().into();
            self.activity.push(crate::activity::ActivityEntry {
                kind,
                chat_title: title,
                body: "Setup script".into(),
                chat_created: SystemTime::now(),
                at: SystemTime::now(),
                unread: true,
            });
            self.persist_activity();
            return false;
        };
        let chat = &mut self.chats[ix];
        std::rc::Rc::make_mut(&mut chat.messages).push(ChatMessage {
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        });
        let entry = crate::activity::ActivityEntry::new(kind, chat, "Setup script".into());
        self.activity.push(entry);
        self.persist_activity();
        ix == self.active
    }
}

#[cfg(test)]
#[path = "setup_script_tests.rs"]
mod setup_script_tests;
