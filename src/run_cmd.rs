//! Run-in-terminal for shell code blocks: the Run button on an assistant
//! `sh`/`bash`/`zsh`/`shell` fence dispatches `RunShellCommand`, which runs
//! the block's code in the chat's working directory and lands the result as
//! a tool-call-style card (command + output + exit code). A suggested
//! command is agent output, so it follows the thread's access mode — modes
//! that auto-approve tool calls run it directly, ask-modes surface the same
//! approval card a backend command request would.
//!
//! The spawn sits behind `CommandRunner` so tests inject a fake — the real
//! `ShellRunner` runs `<shell> -c <command>` and captures stdout/stderr.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::*;

use crate::backend::{ApprovalCard, ApprovalDecision, ApprovalKind, ApprovalResponder};
use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

/// The Run button on a shell code block — carries the block's code and the
/// resolved shell. Dispatched on the window so the workspace's `on_action`
/// runs it; `no_json` — it is never built from a keymap.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(no_json)]
pub struct RunShellCommand {
    /// The code block's contents, run verbatim.
    pub command: String,
    /// The shell binary the block's language tag resolved to.
    pub shell: &'static str,
}

/// The shell binary a fenced block's language tag maps to — `None` for
/// non-shell blocks, which get no Run button.
pub(crate) fn shell_for(lang: &str) -> Option<&'static str> {
    for (tag, shell) in [("sh", "sh"), ("shell", "sh"), ("bash", "bash"), ("zsh", "zsh")] {
        if lang.eq_ignore_ascii_case(tag) {
            return Some(shell);
        }
    }
    None
}

/// What a finished command produced — `code` is `None` when the process
/// never reported one (spawn failure, killed by signal).
#[derive(Clone, Debug)]
pub(crate) struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

/// How a shell command actually runs — the real impl spawns a process;
/// tests substitute a fake via `set_command_runner`.
pub(crate) trait CommandRunner: Send + Sync {
    fn run(&self, shell: &str, command: &str, cwd: &std::path::Path) -> CommandOutput;
}

/// Runs `shell -c command` in `cwd`, capturing both streams.
struct ShellRunner;

impl CommandRunner for ShellRunner {
    fn run(&self, shell: &str, command: &str, cwd: &std::path::Path) -> CommandOutput {
        match std::process::Command::new(shell).arg("-c").arg(command).current_dir(cwd).output() {
            Ok(out) => CommandOutput {
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                code: out.status.code(),
            },
            Err(e) => CommandOutput {
                stdout: String::new(),
                stderr: format!("failed to run {shell}: {e}"),
                code: None,
            },
        }
    }
}

/// The process-wide runner — swapped for a fake in tests.
static RUNNER: std::sync::LazyLock<parking_lot::RwLock<Arc<dyn CommandRunner>>> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(Arc::new(ShellRunner)));

/// The active runner — a fake in tests, `ShellRunner` otherwise.
fn command_runner() -> Arc<dyn CommandRunner> {
    RUNNER.read().clone()
}

/// Install the runner used by every subsequent command run — tests only.
#[cfg(test)]
pub(crate) fn set_command_runner(runner: Arc<dyn CommandRunner>) {
    *RUNNER.write() = runner;
}

/// `tool_ix` space for local command-run cards — counts down from
/// `usize::MAX` so it can't collide with the backends' hashed item ids.
static NEXT_RUN_IX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// A command run in flight or awaiting its approval answer — the chat it
/// belongs to, the code, and the shell it runs under.
struct PendingRun {
    chat_id: u64,
    command: String,
    shell: &'static str,
}

impl Workspace {
    /// The Run button on a shell code block: run `command` in the active
    /// chat's working directory and land the result as a tool card. Ask
    /// modes gate it behind the same approval card a backend command
    /// request uses; auto modes (and a session "Always allow") run it
    /// directly.
    pub(crate) fn run_command_block(&mut self, command: String, shell: &'static str, cx: &mut Context<Self>) {
        let chat_id = self.chats[self.active].id;
        let access = self.chats[self.active].access.unwrap_or(self.access);
        let run = PendingRun { chat_id, command, shell };
        if access.auto_allows() || self.run_approved || self.approval_rule_allows(ApprovalKind::Command, &run.command) {
            self.spawn_command_run(run, cx);
            return;
        }
        let (respond, rx) = std::sync::mpsc::channel::<ApprovalDecision>();
        let respond: ApprovalResponder = respond;
        Rc::make_mut(&mut self.chats[self.active].messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::Assistant,
            kind: MessageKind::Approval(ApprovalCard {
                request_ix: NEXT_RUN_IX.fetch_sub(1, std::sync::atomic::Ordering::Relaxed),
                kind: ApprovalKind::Command,
                detail: run.command.clone().into(),
                decision: None,
                auto_approved: false,
                respond: Some(respond),
            }),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        });
        if self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        self.record_approval(chat_id, ApprovalKind::Command, &run.command);
        cx.notify();
        self.save();
        // Poll like the backend event pump — a blocking recv would park the
        // test executor, which is the same clock the approval click needs.
        cx.spawn(async move |this, cx| {
            let decision = poll_run_decision(&rx, cx).await;
            let _ = this.update(cx, |this, cx| this.answer_command_run(run, decision, cx));
        })
        .detach();
    }

    /// Apply the approval card's answer: `Approve` runs the command,
    /// `ApproveForSession` also blesses later runs this session; `Deny` or
    /// a dropped responder (turn stopped, chat reloaded) leaves the card's
    /// recorded outcome as the only trace.
    fn answer_command_run(&mut self, run: PendingRun, decision: Option<ApprovalDecision>, cx: &mut Context<Self>) {
        match decision {
            Some(ApprovalDecision::ApproveForSession) => self.run_approved = true,
            Some(ApprovalDecision::Approve) => {},
            _ => return,
        }
        self.spawn_command_run(run, cx);
    }

    /// Push the running tool card for `command` and run it on the
    /// background executor; the result lands via `land_command_run`.
    fn spawn_command_run(&mut self, run: PendingRun, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == run.chat_id) else { return };
        let cwd = crate::worktree::workdir_for(chat, self.project.root());
        let tool_ix = NEXT_RUN_IX.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        Rc::make_mut(&mut chat.messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::Assistant,
            kind: MessageKind::Tool(ToolCall {
                tool_ix,
                name: "shell".into(),
                detail: run.command.clone().into(),
                output: String::new().into(),
                status: ToolStatus::Running,
                expanded: true,
            }),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        });
        if self.chats[self.active].id == run.chat_id && self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        cx.notify();
        self.save();
        let runner = command_runner();
        let chat_id = run.chat_id;
        cx.spawn(async move |this, cx| {
            let out = cx.background_executor().spawn(async move { runner.run(run.shell, &run.command, &cwd) }).await;
            let _ = this.update(cx, |this, cx| this.land_command_run(chat_id, tool_ix, out, cx));
        })
        .detach();
    }

    /// Land a finished command's output on its tool card — the card is
    /// found by `tool_ix`, so a truncated or deleted chat just drops it.
    fn land_command_run(&mut self, chat_id: u64, tool_ix: usize, out: CommandOutput, cx: &mut Context<Self>) {
        let is_active = self.chats[self.active].id == chat_id;
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        let Some(pos) = chat.messages.iter().rposition(|m| matches!(&m.kind, MessageKind::Tool(t) if t.tool_ix == tool_ix)) else {
            return;
        };
        if let MessageKind::Tool(t) = &mut Rc::make_mut(&mut chat.messages)[pos].kind {
            t.output = format_output(&out).into();
            t.status = if out.code == Some(0) { ToolStatus::Done } else { ToolStatus::Failed };
        }
        if is_active {
            let sp = self.filtered_pos(pos, cx);
            self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
        } else {
            chat.unread = true;
        }
        crate::dock_badge::update(cx);
        cx.notify();
        self.save();
    }
}

/// Poll the approval channel until a decision lands or the responder drops.
/// A blocking `recv` would park the test executor — the same clock the
/// approval click needs — so this yields on a 30ms timer like the backend
/// event pump. `Some(d)` = answered; `None` = responder dropped.
async fn poll_run_decision(rx: &std::sync::mpsc::Receiver<ApprovalDecision>, cx: &mut gpui_kit::AsyncApp) -> Option<ApprovalDecision> {
    use std::sync::mpsc::TryRecvError::{Disconnected, Empty};
    loop {
        match rx.try_recv() {
            Ok(d) => return Some(d),
            Err(Disconnected) => return None,
            Err(Empty) => cx.background_executor().timer(Duration::from_millis(30)).await,
        }
    }
}

/// The card's output body: stdout, then stderr, then the exit line.
fn format_output(out: &CommandOutput) -> String {
    use std::fmt::Write;
    let mut s = out.stdout.trim_end().to_string();
    if !out.stderr.trim().is_empty() {
        if !s.is_empty() {
            s.push('\n');
        }
        let _ = write!(s, "{}", out.stderr.trim_end());
    }
    match out.code {
        Some(code) => {
            let _ = write!(s, "{}[exit {code}]", if s.is_empty() { "" } else { "\n" });
        },
        None => {
            let _ = write!(s, "{}[no exit code]", if s.is_empty() { "" } else { "\n" });
        },
    }
    s
}
