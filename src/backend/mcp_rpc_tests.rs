//! MCP wire-format tests: request builders, server-request replies, and
//! the listings → pseudo-model / call-target mapping.

use serde_json::{Value, json};

use super::mcp_rpc as wire;

#[test]
fn initialize_and_initialized_shapes() {
    let req = wire::initialize_req(1);
    assert_eq!(req["method"], json!("initialize"));
    assert_eq!(req["params"]["protocolVersion"], json!(wire::PROTOCOL_VERSION));
    // No client capabilities — the server can't ask us for sampling/roots.
    assert_eq!(req["params"]["capabilities"], json!({}));
    assert_eq!(wire::initialized_note()["method"], json!("notifications/initialized"));
}

#[test]
fn list_call_get_shapes() {
    assert_eq!(wire::list_req(2, "tools/list", None)["params"], json!({}));
    assert_eq!(wire::list_req(3, "tools/list", Some("c1"))["params"], json!({"cursor": "c1"}));
    let call = wire::call_req(4, "read", &json!({"path": "/x"}));
    assert_eq!(call["params"], json!({"name": "read", "arguments": {"path": "/x"}}));
    let get = wire::get_req(5, "review", &json!({"code": "fn f()"}));
    assert_eq!(get["method"], json!("prompts/get"));
    assert_eq!(get["params"]["name"], json!("review"));
}

#[test]
fn server_requests_get_a_reply() {
    let ping = wire::server_reply(&json!({"id": 9, "method": "ping"}));
    assert_eq!(ping["result"], json!({}));
    let sampling = wire::server_reply(&json!({"id": 9, "method": "sampling/createMessage"}));
    assert_eq!(sampling["error"]["code"], json!(-32601));
}

fn listings(tools: Vec<Value>, prompts: Vec<Value>) -> wire::Listings {
    wire::Listings { server: "fs".into(), version: "1.0".into(), tools, prompts }
}

#[test]
fn tools_and_prompts_become_models() {
    let l = listings(
        vec![
            json!({"name": "read_file", "description": "Read a file", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}}),
        ],
        vec![json!({"name": "review", "description": "Review code"})],
    );
    let models = l.models();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id.as_str(), "tool:read_file");
    assert_eq!(models[0].label.as_str(), "read_file");
    assert_eq!(models[1].id.as_str(), "prompt:review");
}

#[test]
fn empty_listing_falls_back_to_server_name() {
    let models = listings(vec![], vec![]).models();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.as_str(), "fs");
}

#[test]
fn resolve_prefixed_bare_and_unknown_models() {
    let l = listings(
        vec![json!({"name": "read", "inputSchema": {"properties": {"path": {"type": "string"}}, "required": ["path"]}})],
        vec![json!({"name": "review", "arguments": [{"name": "code", "required": true}]})],
    );
    // Prefixed ids are authoritative.
    match l.resolve("tool:read", "show me") {
        wire::Target::Tool { name, args } => {
            assert_eq!(name, "read");
            assert_eq!(args, json!({"path": "show me"}));
        },
        _ => panic!("expected tool"),
    }
    match l.resolve("prompt:review", "this diff") {
        wire::Target::Prompt { name, args } => {
            assert_eq!(name, "review");
            assert_eq!(args, json!({"code": "this diff"}));
        },
        _ => panic!("expected prompt"),
    }
    // Bare names resolve tool-first, then prompt.
    assert!(matches!(l.resolve("read", "x"), wire::Target::Tool { .. }));
    assert!(matches!(l.resolve("review", "x"), wire::Target::Prompt { .. }));
    // Unknown ids fall back to the server-info pseudo-model.
    assert!(matches!(l.resolve("fs", "x"), wire::Target::Server));
}

/// The `arguments` a resolved target would send — panics on `Server`.
fn args_of(l: &wire::Listings, model: &str) -> Value {
    match l.resolve(model, "hi") {
        wire::Target::Tool { args, .. } | wire::Target::Prompt { args, .. } => args,
        wire::Target::Server => panic!("expected a call target"),
    }
}

#[test]
fn tool_args_pick_the_required_string_property() {
    let l = listings(
        vec![
            json!({"name": "a", "inputSchema": {"properties": {"n": {"type": "number"}, "q": {"type": "string"}}, "required": ["q"]}}),
            json!({"name": "b", "inputSchema": {"properties": {"q": {"type": "string"}}}}),
            json!({"name": "c"}),
        ],
        vec![],
    );
    // Required string wins over an earlier optional string.
    assert_eq!(args_of(&l, "a"), json!({"q": "hi"}));
    // No required props → first string property.
    assert_eq!(args_of(&l, "b"), json!({"q": "hi"}));
    // No schema → the conventional `prompt` key.
    assert_eq!(args_of(&l, "c"), json!({"prompt": "hi"}));
}
