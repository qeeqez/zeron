//! MCP backend tests: `backend_for` construction and the full session
//! driven by canned NDJSON — no real subprocess is ever spawned. Wire
//! builders and the listings → model mapping live in `mcp_rpc_tests`.

use serde_json::{Value, json};

use super::mcp::{McpTurn, drive};
use super::mcp_session::SessionErr;
use super::steer::SharedBuf;
use super::{AgentBackend, AgentEvent};

// ---- backend_for / provider plumbing ----

#[test]
fn backend_for_builds_mcp() {
    let p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::Mcp, "MCP".into());
    let b = crate::backend::backend_for(&p);
    assert_eq!(b.name(), "mcp");
}

#[test]
fn empty_command_fails_the_send_fast() {
    let b = super::mcp::McpBackend::new(String::new(), Vec::new());
    let ctx = super::TurnContext::at(std::path::PathBuf::from("/tmp"), super::AccessMode::Auto);
    let stream = b.send("hi", "tool:x", "Agent", &ctx);
    let events: Vec<AgentEvent> = stream.events.iter().collect();
    assert!(matches!(&events[0], AgentEvent::Error(e) if e.contains("no server command")));
}

#[test]
fn instance_env_lands_on_the_spawned_command() {
    let mut turn = McpTurn::for_test("m1");
    turn.env = vec![("MCP_KEY".into(), "v".into()), (String::new(), "skipped".into())];
    let cmd = super::mcp::build_command(&turn);
    let envs: Vec<_> = cmd
        .get_envs()
        .map(|(k, v)| (k.to_str().unwrap().to_string(), v.unwrap().to_str().unwrap().to_string()))
        .collect();
    assert!(envs.contains(&("MCP_KEY".to_string(), "v".to_string())));
    assert!(!envs.iter().any(|(k, _)| k.is_empty()));
}

// ---- full session over canned NDJSON ----

/// Drive `drive` against a scripted server stdout; returns (result,
/// requests the script saw on stdin, events emitted).
fn run(server_out: &[Value], turn: &McpTurn) -> (Result<(), SessionErr>, Vec<Value>, Vec<AgentEvent>) {
    let input = server_out.iter().map(|v| format!("{v}\n")).collect::<String>();
    let buf = SharedBuf(std::sync::Arc::new(parking_lot::Mutex::new(vec![])));
    let (tx, rx) = std::sync::mpsc::channel();
    let end = drive(turn, input.as_bytes(), SharedBuf(buf.0.clone()), &tx);
    let text = String::from_utf8(buf.0.lock().clone()).unwrap();
    (end, text.lines().map(|l| serde_json::from_str(l).unwrap()).collect(), rx.try_iter().collect())
}

fn init_reply() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "result": {
        "protocolVersion": "2025-06-18",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "fs", "version": "1.0"},
    }})
}

fn tools_reply(tools: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "result": {"tools": tools}})
}

#[test]
fn session_runs_handshake_list_and_call() {
    let server_out = [
        init_reply(),
        tools_reply(json!([{"name": "read", "inputSchema": {"properties": {"path": {"type": "string"}}, "required": ["path"]}}])),
        json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1, "total": 2, "message": "halfway"}}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"content": [{"type": "text", "text": "file contents"}]}}),
    ];
    let turn = McpTurn::for_test("tool:read");
    let (end, reqs, events) = run(&server_out, &turn);
    assert_eq!(end, Ok(()));

    let methods: Vec<&str> = reqs.iter().filter_map(|r| r["method"].as_str()).collect();
    assert_eq!(methods, ["initialize", "notifications/initialized", "tools/list", "tools/call"]);
    assert_eq!(reqs[3]["params"]["name"], json!("read"));
    // The prompt landed on the tool's required string property.
    assert_eq!(reqs[3]["params"]["arguments"], json!({"path": "hi"}));

    assert!(matches!(&events[0], AgentEvent::ToolCallStart { name, .. } if name == "read"));
    // The progress notification landed on the live card.
    assert!(matches!(&events[1], AgentEvent::ToolCallDelta { output, .. } if output.contains("halfway")));
    assert!(matches!(&events[2], AgentEvent::TextDelta(t) if t == "file contents"));
    assert!(matches!(events[3], AgentEvent::ToolCallEnd { ok: true, .. }));
    assert!(matches!(events[4], AgentEvent::Done));
    // The listing refreshed the shared catalog.
    assert_eq!(turn.models.lock()[0].id.as_str(), "tool:read");
}

#[test]
fn tool_error_result_fails_the_card_not_the_turn() {
    let server_out = [
        init_reply(),
        tools_reply(json!([{"name": "read"}])),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"isError": true, "content": [{"type": "text", "text": "no such file"}]}}),
    ];
    let (end, _, events) = run(&server_out, &McpTurn::for_test("tool:read"));
    assert_eq!(end, Ok(()));
    assert!(matches!(&events[1], AgentEvent::TextDelta(t) if t == "no such file"));
    assert!(matches!(events[2], AgentEvent::ToolCallEnd { ok: false, .. }));
    assert!(matches!(events[3], AgentEvent::Done));
}

#[test]
fn prompt_model_calls_prompts_get() {
    let server_out = [
        json!({"jsonrpc": "2.0", "id": 1, "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"prompts": {}},
            "serverInfo": {"name": "prompts", "version": "1.0"},
        }}),
        json!({"jsonrpc": "2.0", "id": 2, "result": {"prompts": [{"name": "review", "arguments": [{"name": "code", "required": true}]}]}}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"messages": [
            {"role": "user", "content": {"type": "text", "text": "Review this:"}},
            {"role": "assistant", "content": {"type": "text", "text": "Looks fine"}},
        ]}}),
    ];
    let (end, reqs, events) = run(&server_out, &McpTurn::for_test("prompt:review"));
    assert_eq!(end, Ok(()));
    assert_eq!(reqs[3]["method"], json!("prompts/get"));
    assert_eq!(reqs[3]["params"]["arguments"], json!({"code": "hi"}));
    // Each template message is its own bubble.
    assert!(matches!(events[1], AgentEvent::TextStart));
    assert!(matches!(&events[2], AgentEvent::TextDelta(t) if t == "Review this:"));
    assert!(matches!(&events[4], AgentEvent::TextDelta(t) if t == "Looks fine"));
    assert!(matches!(events[5], AgentEvent::ToolCallEnd { ok: true, .. }));
    assert!(matches!(events[6], AgentEvent::Done));
}

#[test]
fn server_without_tools_answers_with_server_info() {
    let server_out = [json!({"jsonrpc": "2.0", "id": 1, "result": {
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "serverInfo": {"name": "bare", "version": "2.1"},
    }})];
    let (end, reqs, events) = run(&server_out, &McpTurn::for_test("bare"));
    assert_eq!(end, Ok(()));
    // No capabilities → no list requests at all.
    let methods: Vec<&str> = reqs.iter().filter_map(|r| r["method"].as_str()).collect();
    assert_eq!(methods, ["initialize", "notifications/initialized"]);
    assert!(matches!(&events[0], AgentEvent::TextDelta(t) if t.contains("bare 2.1")));
    assert!(matches!(events[1], AgentEvent::Done));
}

#[test]
fn server_requests_are_answered_mid_turn() {
    let server_out = [
        init_reply(),
        tools_reply(json!([{"name": "slow"}])),
        // The server pings us mid-call; an unknown request gets -32601.
        json!({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"}),
        json!({"jsonrpc": "2.0", "id": "srv-2", "method": "roots/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"content": [{"type": "text", "text": "done"}]}}),
    ];
    let (end, reqs, _) = run(&server_out, &McpTurn::for_test("tool:slow"));
    assert_eq!(end, Ok(()));
    let ping = reqs.iter().find(|r| r["id"] == json!("srv-1")).unwrap();
    assert_eq!(ping["result"], json!({}));
    let roots = reqs.iter().find(|r| r["id"] == json!("srv-2")).unwrap();
    assert_eq!(roots["error"]["code"], json!(-32601));
}

#[test]
fn rpc_error_and_eof_fail_the_turn() {
    // A JSON-RPC error on tools/call surfaces as Failed.
    let server_out = [
        init_reply(),
        tools_reply(json!([{"name": "read"}])),
        json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32602, "message": "bad args"}}),
    ];
    let (end, _, _) = run(&server_out, &McpTurn::for_test("tool:read"));
    assert_eq!(end, Err(SessionErr::Failed("mcp: bad args".into())));

    // EOF before the call's response is Eof, not a protocol error.
    let server_out = [init_reply(), tools_reply(json!([{"name": "read"}]))];
    let (end, _, _) = run(&server_out, &McpTurn::for_test("tool:read"));
    assert_eq!(end, Err(SessionErr::Eof));
}

#[test]
fn tools_list_paginates() {
    let server_out = [
        init_reply(),
        json!({"jsonrpc": "2.0", "id": 2, "result": {"tools": [{"name": "a"}], "nextCursor": "p2"}}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"tools": [{"name": "b"}]}}),
        json!({"jsonrpc": "2.0", "id": 4, "result": {"content": [{"type": "text", "text": "ok"}]}}),
    ];
    let (end, reqs, _) = run(&server_out, &McpTurn::for_test("tool:b"));
    assert_eq!(end, Ok(()));
    let methods: Vec<&str> = reqs.iter().filter_map(|r| r["method"].as_str()).collect();
    assert_eq!(methods, ["initialize", "notifications/initialized", "tools/list", "tools/list", "tools/call"]);
    assert_eq!(reqs[3]["params"]["cursor"], json!("p2"));
}
