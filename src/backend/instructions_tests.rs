//! Backend wire tests for custom instructions: claude's
//! `--append-system-prompt` flag and ACP's `session/prompt` prefix — the
//! codex `developerInstructions` assertions live in `codex_tests.rs` next
//! to the handshake harness. No real subprocess is ever spawned.

use serde_json::json;

use super::acp::{AcpTurn, PumpEnd};
use super::acp_rpc_tests::session_result;
use super::acp_tests::drive;
use super::claude::{ClaudeTurn, build_command};
use super::{AccessMode, AgentEvent};

fn claude_turn(mode: &str, access: AccessMode) -> ClaudeTurn {
    ClaudeTurn {
        prompt: "hi".into(),
        model: "sonnet".into(),
        mode: mode.into(),
        access,
        cwd: std::path::PathBuf::from("/tmp/thread-wt"),
        instructions: None,
        env: Vec::new(),
        slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
        cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

#[test]
fn claude_appends_instructions_to_the_system_prompt() {
    let mut t = claude_turn("Agent", AccessMode::Auto);
    t.instructions = Some("be terse".into());
    let args: Vec<_> = build_command(&t).get_args().map(|a| a.to_os_string()).collect();
    let ix = args.iter().position(|a| a == "--append-system-prompt").expect("args were: {args:?}");
    assert_eq!(args[ix + 1], std::ffi::OsString::from("be terse"));
    // No instructions → no flag.
    let args: Vec<_> = build_command(&claude_turn("Agent", AccessMode::Auto))
        .get_args()
        .map(|a| a.to_os_string())
        .collect();
    assert!(!args.iter().any(|a| a == "--append-system-prompt"));
}

#[test]
fn acp_prompt_carries_instructions_as_a_prefix() {
    let agent_out = [
        json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1, "agentCapabilities": {}}}),
        json!({"jsonrpc": "2.0", "id": 2, "result": session_result()}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"configOptions": []}}),
        json!({"jsonrpc": "2.0", "id": 4, "result": {}}),
        json!({"jsonrpc": "2.0", "id": 5, "result": {"stopReason": "end_turn"}}),
    ];
    let mut turn = AcpTurn::for_test("m2", "Plan", AccessMode::Auto);
    turn.instructions = Some("be terse".into());
    let (end, reqs, _events): (_, _, Vec<AgentEvent>) = drive(&agent_out, &turn);
    assert_eq!(end, PumpEnd::Done);
    let prompt = reqs.iter().find(|r| r["method"] == "session/prompt").expect("no session/prompt sent");
    let text = prompt["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("<system_instructions>\nbe terse\n</system_instructions>\n\nhi"), "prompt was: {text}");
}
