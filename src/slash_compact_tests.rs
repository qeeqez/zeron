//! Headless tests for `/compact`: the prompt fallback sends the transcript
//! for a handoff summary, a backend with native compaction gets no prompt
//! turn, and an empty transcript earns a note instead of a send.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::TestAppContext;

use super::{PromptBackend, mount, seed, submit, texts};
use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::model::{MessageKind, Role};

#[gpui_kit::test]
fn compact_sends_summarization_prompt(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(PromptBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    seed(&workspace, cx, 4);
    submit(&workspace, cx, "/compact");
    cx.run_until_parked();
    let sent = prompts.lock().clone();
    // The compact prompt is the only summarization turn — a second send is
    // the title generator's own prompt.
    let compact: Vec<&String> = sent.iter().filter(|p| p.contains("compact context handoff")).collect();
    assert_eq!(compact.len(), 1, "/compact must send exactly one summarization turn: {sent:?}");
    assert!(compact[0].contains("message 0") && compact[0].contains("message 3"), "prompt must carry the transcript: {}", compact[0]);
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        // The transcript stays intact — the command shows as a compact
        // user row, not the whole prompt.
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[4], "/compact");
    });
}

#[gpui_kit::test]
fn compact_empty_transcript_notes_nothing_to_do(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(PromptBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    submit(&workspace, cx, "/compact");
    cx.run_until_parked();
    assert!(!prompts.lock().iter().any(|p| p.contains("compact context handoff")), "an empty transcript must not reach the backend");
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("Nothing to compact"));
    });
}

/// A backend that answers `compact` with a canned stream — the native
/// path must not send a prompt turn.
struct CompactBackend {
    prompts: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for CompactBackend {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        self.prompts.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn compact(&self, _ctx: &TurnContext) -> Option<ReplyStream> {
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::ToolCallStart { ix: 7, name: "compact".into(), detail: "Summarizing".into() });
        let _ = tx.send(AgentEvent::ToolCallEnd { ix: 7, ok: true });
        let _ = tx.send(AgentEvent::TextStart);
        let _ = tx.send(AgentEvent::TextDelta("**Context compacted**.".into()));
        let _ = tx.send(AgentEvent::Done);
        Some(ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
}

#[gpui_kit::test]
fn compact_uses_backend_when_supported(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(CompactBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    seed(&workspace, cx, 4);
    submit(&workspace, cx, "/compact");
    cx.run_until_parked();
    // The title generator's own send may land — what must not is the
    // summarization fallback prompt.
    assert!(!prompts.lock().iter().any(|p| p.contains("compact context handoff")), "native compaction must not send a prompt turn");
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        assert!(msgs.iter().any(|t| t.contains("Context compacted")), "the compaction result must land: {msgs:?}");
        // No user message — the compact turn isn't a prompt.
        assert!(
            !ws.chats[ws.active]
                .messages
                .iter()
                .any(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t == "/compact"))
        );
    });
}
