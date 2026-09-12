use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;

use crate::model::{Agent, AgentStatus, ChatMessage, DiffCard, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

const REPLY_OK: &str = "Done. The build is **green** — `0 warnings`, all checks passed.\n\n- `cargo build --locked` finished in 3.6s\n- clippy: clean\n- nextest: 0 tests";
const REPLY_FAIL: &str = "The command **failed** — see the tool output above.\n\n```\nerror[E0308]: mismatched types\n```";
const TOOL_OUTPUT: &str =
    "$ cargo build --locked\n   Compiling rixlcode v0.1.0\n    Finished `dev` profile [optimized + debuginfo] target(s) in 3.68s";

pub(crate) struct AgentSpec {
    pub name: &'static str,
    pub lane: &'static str,
    pub steps_total: usize,
}

/// Simulated agent reply: a tool call that runs, a diff card, then a
/// streamed text answer. Replaced by the real backend event stream later
/// (docs/todo/backend.md).
pub fn simulate_reply(this: &mut Workspace, cx: &mut Context<Workspace>) {
    let chat_ix = this.active;
    this.spawn_agent(AgentSpec { name: "explorer", lane: "rixl/explore", steps_total: 4 }, cx);
    this.spawn_agent(AgentSpec { name: "reviewer", lane: "rixl/review", steps_total: 3 }, cx);
    this.chats[chat_ix].messages.push(ChatMessage {
        role: Role::Assistant,
        kind: MessageKind::Tool(ToolCall {
            name: "shell".into(),
            detail: "cargo build --locked".into(),
            output: "".into(),
            status: ToolStatus::Running,
            expanded: false,
        }),
        rating: None,
        usage: None,
        at: SystemTime::now(),
    });
    this.scroller.update(cx, |s, cx| {
        s.append(1, cx);
    });
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
        cx.background_executor().timer(Duration::from_millis(900)).await;
        let _ = this.update(cx, |this, cx| this.begin_stream(chat_ix, cx));
        for _ in 0..12 {
            cx.background_executor().timer(Duration::from_millis(80)).await;
            let _ = this.update(cx, |this, cx| this.stream_chunk(chat_ix, cx));
        }
        let _ = this.update_in(cx, |this, window, cx| {
            this.finish_stream(chat_ix, cx);
            this.notify_done(chat_ix, window, cx);
        });
    });
    this.chats[chat_ix].reply_task = Some(task);
}

impl Workspace {
    /// Spawn a simulated subagent that walks its steps on a timer.
    pub(crate) fn spawn_agent(&mut self, spec: AgentSpec, cx: &mut Context<Self>) {
        let AgentSpec { name, lane, steps_total } = spec;
        self.agents.push(Agent::new(name, lane, steps_total));
        let ix = self.agents.len() - 1;
        cx.notify();
        let task = cx.spawn(async move |this, cx| {
            for step in 1..=steps_total {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                let _ = this.update(cx, |this, cx| this.advance_agent(ix, step, cx));
            }
        });
        self.agents[ix].task = Some(task);
    }

    fn advance_agent(&mut self, ix: usize, step: usize, cx: &mut Context<Self>) {
        let agent = &mut self.agents[ix];
        agent.steps_done = step;
        agent.elapsed_secs += 1;
        agent.step = format!("step {step}").into();
        let tool = match step % 4 {
            1 => "read_file",
            2 => "grep",
            3 => "edit",
            _ => "bash",
        };
        agent.log.push(format!("[{}s] {tool} step {step}", agent.elapsed_secs).into());
        if step == agent.steps_total {
            agent.status = if Self::reply_failed() { AgentStatus::Failed } else { AgentStatus::Done };
            agent.step = "finished".into();
            agent.log.push(format!("[{}s] {}", agent.elapsed_secs, agent.status).into());
        }
        cx.notify();
    }
}

impl Workspace {
    fn reply_failed() -> bool {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() % 4 == 0).unwrap_or(false)
    }

    fn begin_stream(&mut self, chat_ix: usize, cx: &mut Context<Self>) {
        let failed = Self::reply_failed();
        let chat = &mut self.chats[chat_ix];
        chat.failed_flag = failed;
        if let Some(last) = chat.messages.last_mut()
            && let MessageKind::Tool(tool) = &mut last.kind
        {
            tool.status = if failed { ToolStatus::Failed } else { ToolStatus::Done };
            tool.output = TOOL_OUTPUT.into();
        }
        if !failed {
            chat.messages.push(ChatMessage {
                role: Role::Assistant,
                kind: MessageKind::Diff(DiffCard {
                    path: "src/main.rs".into(),
                    added: 24,
                    removed: 6,
                    hunks: "@@ -10,6 +10,24 @@\n fn main() {\n-    println!(\"old\");\n+    gpui_kit::application().run(|cx| {\n+        gpui_kit::init(cx);\n+    });\n }".into(),
                    expanded: false,
                }),
                rating: None,
                usage: None,
                at: SystemTime::now(),
            });
        }
        chat.messages.push(ChatMessage {
            role: Role::Assistant,
            kind: MessageKind::Text("".into()),
            rating: None,
            usage: None,
            at: SystemTime::now(),
        });
        self.scroller.update(cx, |s, cx| {
            s.append(if failed { 1 } else { 2 }, cx);
        });
        cx.notify();
    }

    fn stream_chunk(&mut self, chat_ix: usize, cx: &mut Context<Self>) {
        let chat = &mut self.chats[chat_ix];
        let Some(last) = chat.messages.last_mut() else { return };
        let MessageKind::Text(text) = &mut last.kind else { return };
        let full: &str = if chat.failed_flag { REPLY_FAIL } else { REPLY_OK };
        let next_len = (text.len() + full.len() / 12 + 1).min(full.len());
        *text = full[..next_len].into();
        let last_ix = chat.messages.len() - 1;
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(last_ix..last_ix + 1, cx);
        });
        cx.notify();
    }

    fn finish_stream(&mut self, chat_ix: usize, cx: &mut Context<Self>) {
        let chat = &mut self.chats[chat_ix];
        chat.running = false;
        // failed_flag survives so the retry banner stays until next send.
        chat.started_at = None;
        if chat_ix != self.active {
            chat.unread = true;
        }
        self.scroller.update(cx, |s, cx| {
            s.remeasure(cx);
        });
        cx.notify();
        self.save();
    }
}
