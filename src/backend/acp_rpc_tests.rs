//! ACP wire-format tests: request builders, `session/new` helpers, and
//! replies to agent-initiated requests (permission, `fs/*`).

use serde_json::{Value, json};

use super::acp_rpc as wire;
use crate::backend::{AccessMode, ApprovalDecision, ApprovalRoute};

#[test]
fn initialize_advertises_fs_caps() {
    let req = wire::initialize_req(1, true);
    assert_eq!(req["jsonrpc"], json!("2.0"));
    assert_eq!(req["method"], json!("initialize"));
    assert_eq!(req["params"]["protocolVersion"], json!(1));
    assert_eq!(req["params"]["clientCapabilities"]["fs"], json!({"readTextFile": true, "writeTextFile": true}));
    // Read-only turns must not offer writes.
    assert_eq!(wire::initialize_req(1, false)["params"]["clientCapabilities"]["fs"]["writeTextFile"], json!(false));
}

#[test]
fn session_new_and_prompt_shapes() {
    let new = wire::session_new_req(2, "/work");
    assert_eq!(new["method"], json!("session/new"));
    assert_eq!(new["params"]["cwd"], json!("/work"));
    assert_eq!(new["params"]["mcpServers"], json!([]));

    let prompt = wire::prompt_req(3, "s1", "hello", &[]);
    assert_eq!(prompt["method"], json!("session/prompt"));
    assert_eq!(prompt["params"]["sessionId"], json!("s1"));
    assert_eq!(prompt["params"]["prompt"][0], json!({"type": "text", "text": "hello"}));
}

#[test]
fn prompt_carries_images_as_resource_links() {
    let images = vec![std::path::PathBuf::from("/tmp/Screen Shot.png")];
    let prompt = wire::prompt_req(3, "s1", "look", &images);
    let blocks = &prompt["params"]["prompt"];
    assert_eq!(blocks[0], json!({"type": "text", "text": "look"}));
    assert_eq!(blocks[1]["type"], json!("resource_link"));
    // Spaces percent-encode so the URI stays valid.
    assert_eq!(blocks[1]["uri"], json!("file:///tmp/Screen%20Shot.png"));
    assert_eq!(blocks[1]["name"], json!("Screen Shot.png"));
    assert_eq!(blocks[1]["mimeType"], json!("image/png"));
}

#[test]
fn mode_and_model_request_shapes() {
    let mode = wire::set_mode_req(4, "s1", "plan");
    assert_eq!(mode["method"], json!("session/set_mode"));
    assert_eq!(mode["params"]["modeId"], json!("plan"));

    let cfg = wire::set_config_req(4, "s1", "model", "m2");
    assert_eq!(cfg["method"], json!("session/set_config_option"));
    assert_eq!(cfg["params"]["configId"], json!("model"));
    assert_eq!(cfg["params"]["value"], json!("m2"));

    let legacy = wire::set_model_req(4, "s1", "m2");
    assert_eq!(legacy["method"], json!("session/set_model"));
    assert_eq!(legacy["params"]["modelId"], json!("m2"));
}

/// A `session/new` result advertising modes and a model config option.
pub(super) fn session_result() -> Value {
    json!({
        "sessionId": "s1",
        "modes": {"currentModeId": "agent", "availableModes": [
            {"id": "agent", "name": "Agent"},
            {"id": "plan", "name": "Plan mode"},
        ]},
        "configOptions": [{
            "id": "model", "name": "Model", "category": "model", "type": "select",
            "currentValue": "m1",
            "options": [{"value": "m1", "name": "Model One"}, {"value": "m2", "name": "Model Two"}],
        }],
    })
}

#[test]
fn cache_models_reads_config_options() {
    let models = parking_lot::Mutex::new(vec![]);
    wire::cache_models(&models, &session_result());
    let got = models.lock();
    assert_eq!(got.len(), 2);
    assert_eq!(got[1].id.as_str(), "m2");
    assert_eq!(got[1].label.as_str(), "Model Two");
}

#[test]
fn cache_models_falls_back_to_legacy() {
    let models = parking_lot::Mutex::new(vec![]);
    wire::cache_models(&models, &json!({"models": {"availableModels": [{"modelId": "old-1", "name": "Old"}]}}));
    assert_eq!(models.lock()[0].id.as_str(), "old-1");
}

#[test]
fn model_request_prefers_config_option() {
    let req = wire::model_request(3, "s1", "m2", &session_result()).unwrap();
    assert_eq!(req["method"], json!("session/set_config_option"));
    assert_eq!(req["params"]["value"], json!("m2"));
}

#[test]
fn model_request_uses_legacy_set_model() {
    let result = json!({"models": {"availableModels": [{"modelId": "m9", "name": "Nine"}]}});
    let req = wire::model_request(3, "s1", "m9", &result).unwrap();
    assert_eq!(req["method"], json!("session/set_model"));
    // Unknown model → no request; the agent keeps its configured model.
    assert!(wire::model_request(3, "s1", "nope", &result).is_none());
    assert!(wire::model_request(3, "s1", "nope", &session_result()).is_none());
}

#[test]
fn mode_pick_maps_chat_modes() {
    let result = session_result();
    assert_eq!(wire::mode_pick("Plan", &result).as_deref(), Some("plan"));
    // "Ask" has no dedicated mode — falls back to the read-only plan mode.
    assert_eq!(wire::mode_pick("Ask", &result).as_deref(), Some("plan"));
    assert_eq!(wire::mode_pick("Agent", &result).as_deref(), Some("agent"));
    // No modes advertised → nothing to set.
    assert!(wire::mode_pick("Plan", &json!({})).is_none());
}

fn permission_msg() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 9, "method": "session/request_permission",
        "params": {"sessionId": "s1", "toolCall": {"toolCallId": "t1"}, "options": [
            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
            {"optionId": "deny", "name": "Deny", "kind": "reject_once"},
        ]},
    })
}

#[test]
fn policy_maps_each_access_mode() {
    let sup = wire::Policy::of("Agent", AccessMode::Supervised, "/tmp".into());
    assert!(!sup.write_fs && matches!(sup.route, ApprovalRoute::Ask));

    // Auto-accept-edits: writes allowed inside the workspace, and
    // permission prompts surface to the user (route Ask).
    let edits = wire::Policy::of("Agent", AccessMode::AutoAcceptEdits, "/tmp".into());
    assert!(edits.write_fs && edits.workspace_only && matches!(edits.route, ApprovalRoute::Ask));

    let auto = wire::Policy::of("Agent", AccessMode::Auto, "/tmp".into());
    assert!(auto.write_fs && auto.workspace_only && matches!(auto.route, ApprovalRoute::Auto(ApprovalDecision::Approve)));

    let full = wire::Policy::of("Agent", AccessMode::FullAccess, "/tmp".into());
    assert!(full.write_fs && !full.workspace_only && matches!(full.route, ApprovalRoute::Auto(ApprovalDecision::Approve)));

    // Read-only chat modes never ask — they deny.
    let plan = wire::Policy::of("Plan", AccessMode::Auto, "/tmp".into());
    assert!(matches!(plan.route, ApprovalRoute::Auto(ApprovalDecision::Deny)));
}

#[test]
fn permission_follows_turn_policy() {
    let allow = wire::Policy::of("Agent", AccessMode::Auto, "/tmp".into());
    let reply = wire::request_reply("session/request_permission", &permission_msg(), &allow);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "allow"}));

    let deny = wire::Policy::of("Plan", AccessMode::Auto, "/tmp".into());
    let reply = wire::request_reply("session/request_permission", &permission_msg(), &deny);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "deny"}));

    // No matching option kind → cancelled rather than a bogus selection.
    let mut msg = permission_msg();
    msg["params"]["options"] = json!([]);
    let reply = wire::request_reply("session/request_permission", &msg, &allow);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "cancelled"}));
}

#[test]
fn permission_answer_maps_decisions_to_option_kinds() {
    let msg = permission_msg();
    let params = &msg["params"];
    let id = msg["id"].clone();

    let reply = wire::permission_answer(id.clone(), params, ApprovalDecision::Approve);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "allow"}));

    let reply = wire::permission_answer(id.clone(), params, ApprovalDecision::Deny);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "deny"}));

    // Always-allow prefers the allow_always kind, falling back to once.
    let mut always = permission_msg();
    always["params"]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"optionId": "always", "name": "Always", "kind": "allow_always"}));
    let reply = wire::permission_answer(id.clone(), &always["params"], ApprovalDecision::ApproveForSession);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "always"}));
    let reply = wire::permission_answer(id, params, ApprovalDecision::ApproveForSession);
    assert_eq!(reply["result"]["outcome"], json!({"outcome": "selected", "optionId": "allow"}));
}

#[test]
fn fs_read_honors_line_window() {
    let dir = std::env::temp_dir().join(format!("rixl-acp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "a\nb\nc\nd\n").unwrap();
    let policy = wire::Policy::of("Agent", AccessMode::Auto, dir.clone());
    let msg = |params| json!({"id": 7, "method": "fs/read_text_file", "params": params});

    let reply = wire::request_reply("fs/read_text_file", &msg(json!({"sessionId": "s", "path": path})), &policy);
    assert_eq!(reply["result"]["content"], json!("a\nb\nc\nd\n"));

    let reply = wire::request_reply("fs/read_text_file", &msg(json!({"sessionId": "s", "path": path, "line": 2, "limit": 2})), &policy);
    assert_eq!(reply["result"]["content"], json!("b\nc"));

    let reply = wire::request_reply("fs/read_text_file", &msg(json!({"sessionId": "s", "path": dir.join("nope")})), &policy);
    assert!(reply.get("error").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fs_write_respects_policy_and_workspace() {
    let dir = std::env::temp_dir().join(format!("rixl-acp-w-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.txt");
    let msg = |path: &std::path::Path| json!({"id": 8, "method": "fs/write_text_file", "params": {"sessionId": "s", "path": path, "content": "hi"}});

    // Read-only turn: writes are refused outright.
    let ro = wire::Policy::of("Plan", AccessMode::Auto, dir.clone());
    assert!(wire::request_reply("fs/write_text_file", &msg(&target), &ro).get("error").is_some());

    // Workspace-write: inside cwd works, outside is refused.
    let ws = wire::Policy::of("Agent", AccessMode::Auto, dir.clone());
    let reply = wire::request_reply("fs/write_text_file", &msg(&target), &ws);
    assert_eq!(reply["result"], json!({}));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");
    let outside = std::path::Path::new("/etc/rixl-nope");
    assert!(wire::request_reply("fs/write_text_file", &msg(outside), &ws).get("error").is_some());

    // Unknown methods get method-not-found so the agent can't hang.
    let reply = wire::request_reply("terminal/create", &json!({"id": 1}), &ws);
    assert_eq!(reply["error"]["code"], json!(-32601));
    let _ = std::fs::remove_dir_all(&dir);
}
