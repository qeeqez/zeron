//! ACP `session/new` MCP tests: configured servers land on the
//! `mcpServers` param — stdio argv + env pairs, http url + headers —
//! and disabled servers never reach the agent.

use serde_json::json;

use super::AccessMode;
use super::acp::{AcpTurn, PumpEnd};
use super::acp_tests::drive;

#[test]
fn session_new_carries_configured_mcp_servers() {
    // init reply → session/new → prompt reply. The turn's configured
    // servers must land on session/new's mcpServers param.
    let agent_out = [
        json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1, "agentCapabilities": {}}}),
        json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"stopReason": "end_turn"}}),
    ];
    let servers = vec![
        crate::mcp::McpServer {
            name: "fs".into(),
            command: "mcp-fs --verbose".into(),
            ..Default::default()
        },
        crate::mcp::McpServer {
            name: "web".into(),
            transport: crate::mcp::McpTransport::Http,
            command: "https://mcp.example.com".into(),
            ..Default::default()
        },
        // Disabled servers never reach the agent.
        crate::mcp::McpServer {
            name: "off".into(),
            command: "off-bin".into(),
            enabled: false,
            ..Default::default()
        },
    ];
    let turn = AcpTurn::for_test("m1", "Agent", AccessMode::Auto).with_mcp(servers);
    let (end, reqs, _) = drive(&agent_out, &turn);
    assert_eq!(end, PumpEnd::Done);
    let session_new = reqs.iter().find(|r| r["method"] == json!("session/new")).unwrap();
    assert_eq!(
        session_new["params"]["mcpServers"],
        json!([
            {"command": "mcp-fs", "args": ["--verbose"], "env": []},
            {"type": "http", "url": "https://mcp.example.com", "headers": []},
        ])
    );
}
