//! Headless UI tests for the agents panel's tool-call surface: task agents
//! and chat-turn rows both render per-tool status with expandable output.
//! Imports stay narrow — `use gpui_kit::*` would shadow `#[test]`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{AgentStatus, ToolStatus};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
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

/// A non-"sim" backend that replays canned events, so `send` takes the
/// real `run_backend` path and its agents-panel row.
struct StubBackend;

impl AgentBackend for StubBackend {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        for e in [
            AgentEvent::ToolCallStart { ix: 0, name: "bash".into(), detail: "cargo test".into() },
            AgentEvent::ToolCallDelta { ix: 0, output: "running 3 tests".into() },
            AgentEvent::ToolCallEnd { ix: 0, ok: true },
            AgentEvent::TextDelta("all green".into()),
            AgentEvent::Done,
        ] {
            let _ = tx.send(e);
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Advance the test clock until the workspace has no running agents (or
/// the budget runs out).
fn settle_agents(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| ws.agents.iter().all(|a| a.status != AgentStatus::Running)) {
            return;
        }
    }
    panic!("agents never finished");
}

#[test]
fn task_agent_shows_tool_rows_with_expandable_output() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(StubBackend);
            this.spawn_task_agent("run the tests".to_string(), cx);
            this.agents_panel_open = true;
        });
    });
    settle_agents(&ws, cx);

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let card = window.find(("agent-card", 0u64));
        assert!(card.visible(), "finished task agent should render a card");
        assert!(window.find("agent-tool-0-0").visible(), "tool call should render as a row");
        assert!(window.try_find("agent-tool-out-0-0").is_none(), "output starts collapsed");

        window.click("agent-tool-0-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("agent-tool-out-0-0").visible(), "click should reveal tool output");
    });
    ws.read_with(cx, |ws, _| {
        let agent = &ws.agents[0];
        assert_eq!(agent.status, AgentStatus::Done);
        assert_eq!(agent.tools.len(), 1);
        assert_eq!(agent.tools[0].status, ToolStatus::Done);
        assert!(agent.tools[0].output.contains("running 3 tests"));
        assert!(agent.expanded_tools.contains(&0));
    });
}

#[test]
fn chat_turn_row_derives_tool_status_from_messages() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(StubBackend);
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
            this.agents_panel_open = true;
        });
    });
    settle_agents(&ws, cx);

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("agent-card", 0u64)).visible(), "chat turn should open an agent row");
        assert!(window.find("agent-tool-0-0").visible(), "chat tool call should appear under the row");
        window.click("agent-tool-0-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("agent-tool-out-0-0").visible(), "chat tool output should expand too");
    });
    ws.read_with(cx, |ws, _| {
        let agent = &ws.agents[0];
        assert_eq!(agent.status, AgentStatus::Done);
        // Finish snapshots the chat's tool calls onto the row so they
        // survive the run_agent link dropping.
        assert_eq!(agent.tools.len(), 1);
        assert_eq!(agent.tools[0].status, ToolStatus::Done);
        assert!(agent.tools[0].output.contains("running 3 tests"));
        assert!(agent.expanded_tools.contains(&0), "expansion keys on tool_ix");
    });
}

#[test]
fn fmt_elapsed_compacts_durations() {
    assert_eq!(crate::agents::fmt_elapsed(0), "0s");
    assert_eq!(crate::agents::fmt_elapsed(59), "59s");
    assert_eq!(crate::agents::fmt_elapsed(60), "1m 0s");
    assert_eq!(crate::agents::fmt_elapsed(75), "1m 15s");
    assert_eq!(crate::agents::fmt_elapsed(3600), "1h 0m");
    assert_eq!(crate::agents::fmt_elapsed(3700), "1h 1m");
}
