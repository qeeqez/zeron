//! Headless tests for the backend reply lifecycle: stopping a turn must
//! signal the backend's cancel flag even when the pump thread never wakes,
//! and the cancelled agent row keeps the tool calls the turn collected.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{AgentStatus, ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-run-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    sandbox_home();
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// A backend whose stream never produces another event — the pump thread
/// stays blocked in `recv`, so only the reply task's cancel guard can
/// signal the backend. `cancelled` is observable from the test.
struct HangingBackend {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl AgentBackend for HangingBackend {
    fn name(&self) -> &'static str {
        "hanging"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        std::mem::forget(tx); // producer never exits — the pump blocks on recv
        ReplyStream { events, child: None, cancelled: self.cancelled.clone() }
    }
}

#[test]
fn stop_reply_signals_backend_cancel() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(HangingBackend { cancelled: cancelled.clone() });
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    cx.run_until_parked();
    assert!(!cancelled.load(std::sync::atomic::Ordering::SeqCst), "turn still running — not cancelled yet");

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.stop_reply(cx));
    });
    cx.run_until_parked();
    assert!(
        cancelled.load(std::sync::atomic::Ordering::SeqCst),
        "stop must set the stream's cancelled flag even when the pump never wakes on an event"
    );
}

/// A backend that emits one running tool call, then goes silent forever.
struct EmitThenHangBackend;

impl AgentBackend for EmitThenHangBackend {
    fn name(&self) -> &'static str {
        "emit-hang"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::ToolCallStart { ix: 0, name: "bash".into(), detail: "sleep 99".into() });
        let _ = tx.send(AgentEvent::ToolCallDelta { ix: 0, output: "partial".into() });
        std::mem::forget(tx);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

#[test]
fn stop_reply_snapshots_tools_onto_agent_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(EmitThenHangBackend);
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    // Wait for the pump thread to forward the two queued events into the
    // reply task. `advance_clock`/`run_until_parked` only drive the fake
    // executor — the `std::thread::spawn` pump runs in real time, so a
    // fixed iteration count can finish before the OS schedules it under
    // parallel test load. Poll the applied state with a real-time
    // deadline instead.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        let applied = ws.read_with(cx, |ws, _| {
            ws.chats[0]
                .messages
                .iter()
                .any(|m| matches!(&m.kind, MessageKind::Tool(t) if t.output.contains("partial")))
        });
        if applied {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "pump thread never delivered the tool events");
        // Deschedule so the OS gives the pump thread a core — yield_now can
        // return immediately when the run queue is full of test threads.
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.stop_reply(cx));
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let agent = &ws.agents[0];
        assert_eq!(agent.status, AgentStatus::Cancelled);
        // The cancelled card keeps the tool rows the turn collected —
        // settled to Failed since the calls never produced a result.
        assert_eq!(agent.tools.len(), 1);
        assert_eq!(agent.tools[0].name.as_ref(), "bash");
        assert_eq!(agent.tools[0].status, ToolStatus::Failed);
        assert!(agent.tools[0].output.contains("partial"));
        // The chat's own tool card stops spinning too.
        let chat = &ws.chats[0];
        let tool = chat
            .messages
            .iter()
            .find_map(|m| match &m.kind {
                MessageKind::Tool(t) => Some(t),
                _ => None,
            })
            .expect("tool message");
        assert_eq!(tool.status, ToolStatus::Failed);
    });
}

fn tool_msg(ix: usize, status: ToolStatus) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Tool(ToolCall {
            tool_ix: ix,
            name: "bash".into(),
            detail: "".into(),
            output: String::new().into(),
            status,
            expanded: false,
        }),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::User,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

#[test]
fn finish_run_agent_settles_only_current_turn() {
    // A still-Running tool from an earlier turn (e.g. persisted mid-turn)
    // must not be rewritten by a later turn's outcome.
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            let chat_id = this.chats[0].id;
            {
                let chat = &mut this.chats[0];
                let msgs = std::rc::Rc::make_mut(&mut chat.messages);
                msgs.push(user_msg("first"));
                msgs.push(tool_msg(0, ToolStatus::Running)); // stale earlier turn
                msgs.push(user_msg("second"));
                msgs.push(tool_msg(1, ToolStatus::Running)); // current turn
            }
            this.spawn_run_agent(crate::agents::RunAgentSpec { chat_id, name: "stub", lane: "m" }, cx);
            this.finish_run_agent(chat_id, true, cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        let msgs = &ws.chats[0].messages;
        assert_eq!(
            match &msgs[1].kind {
                MessageKind::Tool(t) => t.status,
                _ => panic!("tool message"),
            },
            ToolStatus::Running,
            "earlier turn's tool keeps its own state"
        );
        assert_eq!(
            match &msgs[3].kind {
                MessageKind::Tool(t) => t.status,
                _ => panic!("tool message"),
            },
            ToolStatus::Done
        );
        let agent = &ws.agents[0];
        assert_eq!(agent.tools.len(), 1);
        assert_eq!(agent.tools[0].tool_ix, 1);
        assert_eq!(agent.tools[0].status, ToolStatus::Done);
    });
}
