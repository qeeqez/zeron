//! ACP backend tests: `backend_for` construction, the `session/update`
//! decoder, and the full pump driven by canned NDJSON — no real
//! subprocess is ever spawned.

use serde_json::{Value, json};

use super::acp::{AcpBackend, AcpTurn, PumpEnd, pump};
use super::acp_decode::AcpDecoder;
use super::acp_rpc_tests::session_result;
use super::{AgentBackend, AgentEvent};

// ---- backend_for / provider plumbing ----

#[test]
fn backend_for_builds_acp() {
    let p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::Acp, "ACP".into());
    let b = crate::backend::backend_for(&p);
    assert_eq!(b.name(), "acp");
}

#[test]
fn empty_command_falls_back_to_default() {
    let b = AcpBackend::new("  ".into());
    assert_eq!(b.name(), "acp");
    assert!(b.models().is_empty());
}

// ---- session/update decoder ----

#[test]
fn message_chunks_open_bubbles_per_message_id() {
    let mut d = AcpDecoder::new();
    let chunk = |mid: &str, text: &str| json!({"sessionUpdate": "agent_message_chunk", "messageId": mid, "content": {"type": "text", "text": text}});
    let e = d.update(&chunk("m1", "Hello"));
    assert!(matches!(e[0], AgentEvent::TextStart));
    assert!(matches!(&e[1], AgentEvent::TextDelta(t) if t == "Hello"));

    // Same messageId keeps appending; a new one opens a fresh bubble.
    let e = d.update(&chunk("m1", " world"));
    assert_eq!(e.len(), 1);
    let e = d.update(&chunk("m2", "next"));
    assert!(matches!(e[0], AgentEvent::TextStart));
}

#[test]
fn thought_chunks_stream_into_thinking_card() {
    let mut d = AcpDecoder::new();
    let e = d.update(&json!({"sessionUpdate": "agent_thought_chunk", "messageId": "t1", "content": {"type": "text", "text": "hmm"}}));
    assert!(matches!(&e[0], AgentEvent::ToolCallStart { name, .. } if name == "thinking"));
    assert!(matches!(&e[1], AgentEvent::ToolCallDelta { output, .. } if output == "hmm"));

    // A message chunk closes the thought card before opening the bubble.
    let e = d.update(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "done"}}));
    assert!(matches!(e[0], AgentEvent::ToolCallEnd { ok: true, .. }));
    assert!(matches!(e[1], AgentEvent::TextStart));
}

#[test]
fn tool_call_lifecycle() {
    let mut d = AcpDecoder::new();
    let e = d.update(&json!({
        "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "cargo build",
        "kind": "execute", "status": "in_progress",
    }));
    assert!(matches!(&e[0], AgentEvent::ToolCallStart { name, detail, .. } if name == "shell" && detail == "cargo build"));

    // Content replaces as a full snapshot.
    let e = d.update(&json!({
        "sessionUpdate": "tool_call_update", "toolCallId": "t1",
        "content": [{"type": "content", "content": {"type": "text", "text": "ok"}}],
    }));
    assert!(matches!(&e[0], AgentEvent::ToolCallSet { output, .. } if output == "ok"));

    let e = d.update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}));
    assert!(matches!(e[0], AgentEvent::ToolCallEnd { ok: true, .. }));
    // Terminal is sticky — a late update emits nothing.
    assert!(
        d.update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}))
            .is_empty()
    );
}

#[test]
fn tool_call_update_opens_unseen_card_and_fails() {
    let mut d = AcpDecoder::new();
    let e = d.update(&json!({
        "sessionUpdate": "tool_call_update", "toolCallId": "t9",
        "title": "read f", "kind": "read", "status": "failed",
    }));
    assert!(matches!(&e[0], AgentEvent::ToolCallStart { name, detail, .. } if name == "read" && detail == "read f"));
    assert!(matches!(e[1], AgentEvent::ToolCallEnd { ok: false, .. }));
}

#[test]
fn tool_diff_content_becomes_diff_card() {
    let mut d = AcpDecoder::new();
    let e = d.update(&json!({
        "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "edit", "kind": "edit",
        "content": [{"type": "diff", "path": "src/a.rs", "oldText": "old\n", "newText": "new\nmore\n"}],
    }));
    let diff = e.iter().find(|e| matches!(e, AgentEvent::Diff { .. })).unwrap();
    let AgentEvent::Diff { path, added, removed, .. } = diff else { unreachable!() };
    assert_eq!(path.as_str(), "src/a.rs");
    assert_eq!((*added, *removed), (2, 1));
}

#[test]
fn plan_replaces_checklist() {
    let mut d = AcpDecoder::new();
    let e = d.update(&json!({"sessionUpdate": "plan", "entries": [
        {"content": "step one", "status": "completed", "priority": "high"},
        {"content": "step two", "status": "in_progress", "priority": "medium"},
        {"content": "step three", "status": "pending", "priority": "low"},
    ]}));
    assert!(matches!(&e[0], AgentEvent::ToolCallStart { name, .. } if name == "plan"));
    assert!(matches!(&e[1], AgentEvent::ToolCallSet { output, .. } if output == "☑ step one\n◐ step two\n☐ step three"));

    // A second plan update reuses the card — no second ToolCallStart.
    let e = d.update(&json!({"sessionUpdate": "plan", "entries": [{"content": "only", "status": "pending", "priority": "low"}]}));
    assert_eq!(e.len(), 1);
    assert!(matches!(&e[0], AgentEvent::ToolCallSet { output, .. } if output == "☐ only"));
}

#[test]
fn usage_update_maps_context_occupancy() {
    let mut d = AcpDecoder::new();
    let e = d.update(&json!({"sessionUpdate": "usage_update", "used": 1200, "size": 200000}));
    assert!(matches!(e[0], AgentEvent::Usage { input: 1200, output: 200000 }));
}

#[test]
fn close_open_ends_unfinished_cards() {
    let mut d = AcpDecoder::new();
    d.update(&json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "x", "status": "in_progress"}));
    d.update(&json!({"sessionUpdate": "plan", "entries": [{"content": "s", "status": "pending", "priority": "low"}]}));
    let e = d.close_open();
    // Plan card + still-running tool both close.
    assert_eq!(e.iter().filter(|e| matches!(e, AgentEvent::ToolCallEnd { .. })).count(), 2);
}

// ---- full pump over canned NDJSON ----

/// Shared write buffer so the test can inspect what the pump sent.
struct SharedBuf(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn sent(buf: &SharedBuf) -> Vec<Value> {
    let text = String::from_utf8(buf.0.lock().clone()).unwrap();
    text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn drive(agent_out: &[Value], turn: &AcpTurn) -> (PumpEnd, Vec<Value>, Vec<AgentEvent>) {
    let input = agent_out.iter().map(|v| format!("{v}\n")).collect::<String>();
    let buf = SharedBuf(std::sync::Arc::new(parking_lot::Mutex::new(vec![])));
    let (tx, rx) = std::sync::mpsc::channel();
    let end = pump(turn, std::io::BufReader::new(input.as_bytes()), Box::new(SharedBuf(buf.0.clone())), &tx);
    (end, sent(&buf), rx.try_iter().collect())
}

#[test]
fn pump_runs_full_handshake_and_streams() {
    // init reply → session/new (modes + model config) → set_config_option
    // reply → set_mode reply → one text chunk → prompt reply. The turn
    // asks for model m2 in Plan mode.
    let agent_out = [
        json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1, "agentCapabilities": {}}}),
        json!({"jsonrpc": "2.0", "id": 2, "result": session_result()}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"configOptions": []}}),
        json!({"jsonrpc": "2.0", "id": 4, "result": {}}),
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "Hi there"}}}}),
        json!({"jsonrpc": "2.0", "id": 5, "result": {"stopReason": "end_turn"}}),
    ];
    let turn = AcpTurn::for_test("m2", "Plan");
    let (end, reqs, events) = drive(&agent_out, &turn);
    assert_eq!(end, PumpEnd::Done);

    let methods: Vec<&str> = reqs.iter().filter_map(|r| r["method"].as_str()).collect();
    assert_eq!(methods, ["initialize", "session/new", "session/set_config_option", "session/set_mode", "session/prompt"]);
    assert_eq!(reqs[2]["params"]["value"], json!("m2"));
    assert_eq!(reqs[3]["params"]["modeId"], json!("plan"));
    assert_eq!(reqs[4]["params"]["prompt"][0]["text"], json!("hi"));

    assert!(matches!(events[0], AgentEvent::TextStart));
    assert!(matches!(&events[1], AgentEvent::TextDelta(t) if t == "Hi there"));
    assert!(matches!(events[2], AgentEvent::Done));
    // The session/new response populated the shared model catalog.
    assert_eq!(turn.models.lock()[1].id.as_str(), "m2");
}

#[test]
fn pump_answers_permission_and_reports_errors() {
    let agent_out = [
        json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}}),
        json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}}),
        json!({"jsonrpc": "2.0", "id": 9, "method": "session/request_permission", "params": {"sessionId": "s1", "toolCall": {"toolCallId": "t"}, "options": [{"optionId": "a", "name": "Allow", "kind": "allow_once"}]}}),
        json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32603, "message": "boom"}}),
    ];
    // Agent mode + workspace-write → permission auto-allowed.
    let turn = AcpTurn::for_test("m1", "Agent");
    let (end, reqs, events) = drive(&agent_out, &turn);
    assert_eq!(end, PumpEnd::Done);

    // No model/mode advertised → straight to session/prompt (id 3).
    let methods: Vec<&str> = reqs.iter().filter_map(|r| r["method"].as_str()).collect();
    assert_eq!(methods, ["initialize", "session/new", "session/prompt"]);
    // The permission request got a selected-allow reply on id 9.
    let reply = reqs.iter().find(|r| r["id"] == json!(9)).unwrap();
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "a"}));

    // The prompt's error response surfaces Error then Done.
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Error(m) if m == "acp: boom")));
    assert!(matches!(events.last(), Some(AgentEvent::Done)));
}

#[test]
fn pump_eof_before_prompt_response() {
    let input = [json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}})];
    let turn = AcpTurn::for_test("m1", "Agent");
    let (end, _, _) = drive(&input, &turn);
    assert_eq!(end, PumpEnd::Eof);
}
