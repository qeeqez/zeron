//! Tests for stream coalescing: consecutive deltas fold into one apply,
//! run-breakers and `Done` pass through untouched, and a queued burst
//! lands as a single bubble through the whole pump→drain→apply path.
//! `AgentEvent` has no `PartialEq` (approval responders aren't
//! comparable), so assertions match on shape instead of `assert_eq!`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::TestAppContext;

use super::coalesce;
use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::backend_run_tests::mount;
use crate::model::MessageKind;

fn text(s: &str) -> AgentEvent {
    AgentEvent::TextDelta(s.into())
}

fn tool(ix: usize, s: &str) -> AgentEvent {
    AgentEvent::ToolCallDelta { ix, output: s.into() }
}

#[test]
fn coalesce_passes_through_empty_and_singletons() {
    assert!(coalesce(Vec::new()).is_empty());
    let out = coalesce(vec![text("a"), AgentEvent::Done]);
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], AgentEvent::TextDelta(t) if t.as_str() == "a"));
    assert!(matches!(&out[1], AgentEvent::Done));
}

#[test]
fn coalesce_merges_consecutive_text_deltas_in_order() {
    let out = coalesce(vec![text("a"), text("b"), text("c")]);
    assert!(matches!(&out[..], [AgentEvent::TextDelta(t)] if t.as_str() == "abc"));
}

#[test]
fn coalesce_run_breaks_on_any_other_event() {
    let out = coalesce(vec![text("a"), text("b"), AgentEvent::TextStart, text("c"), text("d")]);
    assert_eq!(out.len(), 3);
    assert!(matches!(&out[0], AgentEvent::TextDelta(t) if t.as_str() == "ab"));
    assert!(matches!(&out[1], AgentEvent::TextStart));
    assert!(matches!(&out[2], AgentEvent::TextDelta(t) if t.as_str() == "cd"));
}

#[test]
fn coalesce_tool_deltas_merge_only_within_same_ix() {
    let out = coalesce(vec![tool(0, "a"), tool(0, "b"), tool(1, "c"), tool(0, "d")]);
    assert_eq!(out.len(), 3);
    assert!(matches!(&out[0], AgentEvent::ToolCallDelta { ix: 0, output } if output.as_str() == "ab"));
    assert!(matches!(&out[1], AgentEvent::ToolCallDelta { ix: 1, output } if output.as_str() == "c"));
    assert!(matches!(&out[2], AgentEvent::ToolCallDelta { ix: 0, output } if output.as_str() == "d"));
}

#[test]
fn coalesce_never_merges_across_kinds() {
    let out = coalesce(vec![text("a"), tool(0, "b"), text("c"), AgentEvent::Done]);
    assert_eq!(out.len(), 4);
    assert!(matches!(&out[0], AgentEvent::TextDelta(t) if t.as_str() == "a"));
    assert!(matches!(&out[1], AgentEvent::ToolCallDelta { ix: 0, output } if output.as_str() == "b"));
    assert!(matches!(&out[2], AgentEvent::TextDelta(t) if t.as_str() == "c"));
    assert!(matches!(&out[3], AgentEvent::Done));
}

/// A backend that queues a whole token burst plus `Done` inside `send` —
/// every event is already in the channel when the reply task first polls,
/// so the drain has to batch them.
struct BurstBackend {
    deltas: usize,
}

impl AgentBackend for BurstBackend {
    fn name(&self) -> &'static str {
        "burst"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        for i in 0..self.deltas {
            let _ = tx.send(AgentEvent::TextDelta(format!("d{i} ").into()));
        }
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[test]
fn queued_burst_lands_as_one_bubble() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = Arc::new(BurstBackend { deltas: 64 });
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    let expected: String = (0..64).map(|i| format!("d{i} ")).collect();
    // The pump is a real thread — poll applied state with a real-time
    // deadline instead of assuming run_until_parked covered it (same
    // pattern as backend_run_tests).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        let landed = ws.read_with(cx, |ws, _| {
            ws.chats[0]
                .messages
                .iter()
                .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.as_str() == expected.as_str()))
        });
        if landed {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "burst never fully applied");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    ws.read_with(cx, |ws, _| {
        // The deltas applied to one in-flight bubble, not 64 — and Done
        // still ended the turn.
        let hits = ws.chats[0]
            .messages
            .iter()
            .filter(|m| matches!(&m.kind, MessageKind::Text(t) if t.as_str() == expected.as_str()))
            .count();
        assert_eq!(hits, 1);
        assert!(!ws.chats[0].running);
    });
}
