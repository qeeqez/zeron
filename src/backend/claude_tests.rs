//! Tests for the `claude -p --output-format stream-json` decoder — pure
//! parsing, no subprocess. Line shapes mirror a captured stream with
//! `--include-partial-messages`.

use super::claude_parse::ClaudeDecoder;
use crate::backend::{AgentBackend, AgentEvent};

fn events(d: &mut ClaudeDecoder, line: &str) -> Vec<AgentEvent> {
    d.line(line).events
}

#[test]
fn text_streams_deltas_then_snapshot_skips() {
    let mut d = ClaudeDecoder::new();
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":1}}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::Usage { input: 12, output: 1 }));

    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
    );
    assert!(matches!(evs[0], AgentEvent::TextStart));

    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "hello"));
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == " world"));

    // The assistant snapshot repeats the full text — must not re-emit.
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello world"}],"usage":{"input_tokens":12,"output_tokens":5}}}"#,
    );
    assert!(evs.iter().all(|e| matches!(e, AgentEvent::Usage { .. })));
}

#[test]
fn assistant_snapshot_without_partials_emits_text() {
    let mut d = ClaudeDecoder::new();
    let evs = events(&mut d, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"full reply"}]}}"#);
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "full reply"));
}

#[test]
fn tool_use_opens_card_and_result_closes_it() {
    let mut d = ClaudeDecoder::new();
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}}}"#,
    );
    let ix = match &evs[0] {
        AgentEvent::ToolCallStart { ix, name, .. } => {
            assert_eq!(name.as_ref(), "Bash");
            *ix
        },
        e => panic!("expected ToolCallStart, got {e:?}"),
    };

    // Snapshot carries the real input — streamed card gets the command
    // summary as its first output line, not a second card.
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { ix: i, output } if *i == ix && output == "cargo test\n"));

    let evs = events(
        &mut d,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"ok 3 passed"}]}]}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { ix: i, .. } if *i == ix));
    assert!(matches!(&evs[1], AgentEvent::ToolCallEnd { ix: i, ok: true } if *i == ix));
}

#[test]
fn tool_use_snapshot_only_and_error_result() {
    let mut d = ClaudeDecoder::new();
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Read","input":{"file_path":"/tmp/x.rs"}}]}}"#,
    );
    let ix = match &evs[0] {
        AgentEvent::ToolCallStart { ix, name, detail } => {
            assert_eq!(name.as_ref(), "Read");
            assert_eq!(detail.as_ref(), "/tmp/x.rs");
            *ix
        },
        e => panic!("expected ToolCallStart, got {e:?}"),
    };

    let evs = events(
        &mut d,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_2","is_error":true,"content":"permission denied"}]}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { ix: i, output } if *i == ix && output == "permission denied"));
    assert!(matches!(&evs[1], AgentEvent::ToolCallEnd { ix: i, ok: false } if *i == ix));
}

#[test]
fn result_success_emits_usage_and_done() {
    let mut d = ClaudeDecoder::new();
    let dec = d.line(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","usage":{"input_tokens":100,"cache_read_input_tokens":50,"output_tokens":20}}"#,
    );
    assert!(dec.turn_over);
    assert!(dec.events.iter().any(|e| matches!(e, AgentEvent::Usage { input: 150, output: 20 })));
    assert!(matches!(dec.events.last(), Some(AgentEvent::Done)));
}

#[test]
fn result_error_subtype_emits_error_then_done() {
    let mut d = ClaudeDecoder::new();
    let dec = d.line(r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"hit the turn limit"}"#);
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::Error(m) if m.contains("error_max_turns") && m.contains("hit the turn limit")));
    assert!(matches!(dec.events.last(), Some(AgentEvent::Done)));
}

#[test]
fn result_closes_tool_cards_left_open() {
    let mut d = ClaudeDecoder::new();
    events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_9","name":"Bash","input":{"command":"sleep 99"}}]}}"#,
    );
    // Turn ends without a tool_result — the card must not spin forever.
    let dec = d.line(r#"{"type":"result","subtype":"success","is_error":false,"result":"stopped"}"#);
    assert!(dec.events.iter().any(|e| matches!(e, AgentEvent::ToolCallEnd { ok: true, .. })));
    assert!(matches!(dec.events.last(), Some(AgentEvent::Done)));
}

#[test]
fn backend_for_builds_claude() {
    let p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::ClaudeCli, "Claude".into());
    assert_eq!(crate::backend::backend_for(&p).name(), "claude-cli");
}

#[test]
fn claude_models_have_no_default_entry() {
    let models = crate::backend::ClaudeCliBackend::new().models();
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| m.id != "default"));
    assert!(models.iter().any(|m| m.id == "sonnet"));
}
