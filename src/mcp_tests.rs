//! MCP config tests: env/command parsing, the `~/.codex/config.toml`
//! `[mcp_servers]` writer + reader, `mcpServerStatus/list` decoding, the
//! ACP `mcpServers` mapping, and settings.json persistence.

use serde_json::json;

use crate::mcp::{McpServer, McpTransport, acp_mcp_servers, parse_env, parse_status_page, split_command};
use crate::mcp_config::parse_codex_config;

fn stdio(name: &str, command: &str) -> McpServer {
    McpServer {
        name: name.into(),
        transport: McpTransport::Stdio,
        command: command.into(),
        ..Default::default()
    }
}

#[test]
fn parse_env_reads_pairs() {
    let env = parse_env("A=1, B=two\nC=x=y");
    assert_eq!(env["A"], "1");
    assert_eq!(env["B"], "two");
    // Values may contain `=`; entries without one are dropped.
    assert_eq!(env["C"], "x=y");
    assert!(parse_env("no-equals-here").is_empty());
}

#[test]
fn split_command_handles_quotes() {
    assert_eq!(split_command("npx -y @scope/pkg"), ["npx", "-y", "@scope/pkg"]);
    assert_eq!(split_command("run \"two words\" tail"), ["run", "two words", "tail"]);
    assert!(split_command("   ").is_empty());
}

#[test]
fn codex_config_round_trip() {
    let dir = std::env::temp_dir().join(format!("rixlcode-mcp-cfg-{}", std::process::id()));
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("CODEX_HOME", &dir) };
    let mut http = stdio("docs", "https://mcp.example.com/sse");
    http.transport = McpTransport::Http;
    http.env.insert("Authorization".into(), "Bearer t".into());
    let mut off = stdio("playwright", "npx @playwright/mcp@latest");
    off.enabled = false;
    let mut env = stdio("fs", "mcp-fs --root /tmp");
    env.env.insert("DEBUG".into(), "1".into());

    crate::mcp_config::write_codex_config(&[env.clone(), http.clone(), off.clone()]);
    let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(text.contains("[mcp_servers.fs]"), "{text}");
    assert!(text.contains("command = \"mcp-fs\""));
    assert!(text.contains("args = [\"--root\", \"/tmp\"]"));
    assert!(text.contains("[mcp_servers.fs.env]"));
    assert!(text.contains("[mcp_servers.docs]"));
    assert!(text.contains("url = \"https://mcp.example.com/sse\""));
    assert!(text.contains("[mcp_servers.docs.http_headers]"));
    assert!(text.contains("[mcp_servers.playwright]"));
    assert!(text.contains("enabled = false"));

    // The reader sees the same servers back.
    let parsed = parse_codex_config(&text);
    assert_eq!(parsed, vec![env, http, off]);
}

#[test]
fn codex_config_preserves_other_settings() {
    let dir = std::env::temp_dir().join(format!("rixlcode-mcp-keep-{}", std::process::id()));
    unsafe { std::env::set_var("CODEX_HOME", &dir) };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "model = \"gpt-5\"\n\n[mcp_servers.old]\ncommand = \"old-bin\"\n\n[other]\nkey = 1\n").unwrap();
    crate::mcp_config::write_codex_config(&[stdio("new", "new-bin")]);
    let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(text.contains("model = \"gpt-5\""), "other settings kept: {text}");
    assert!(text.contains("[other]"), "other tables kept: {text}");
    assert!(!text.contains("mcp_servers.old"), "stale server dropped: {text}");
    assert!(text.contains("[mcp_servers.new]"));
}

#[test]
fn parse_codex_config_reads_real_world_shape() {
    let text = r#"
model = "gpt-5"

[mcp_servers.playwright]
args = ["@playwright/mcp@latest"]
command = "npx"
enabled = false

[mcp_servers.node_repl]
command = "/opt/node_repl"
startup_timeout_sec = 120

[mcp_servers.node_repl.env]
NODE_PATH = "/opt/lib"

[mcp_servers."my server"]
url = "https://example.com/mcp"
"#;
    let servers = parse_codex_config(text);
    assert_eq!(servers.len(), 3);
    assert_eq!(servers[0].name, "playwright");
    assert_eq!(servers[0].command, "npx @playwright/mcp@latest");
    assert!(!servers[0].enabled);
    assert_eq!(servers[1].name, "node_repl");
    assert_eq!(servers[1].env["NODE_PATH"], "/opt/lib");
    assert!(servers[1].enabled, "absent enabled defaults true");
    assert_eq!(servers[2].name, "my server");
    assert_eq!(servers[2].transport, McpTransport::Http);
    assert_eq!(servers[2].command, "https://example.com/mcp");
}

#[test]
fn save_mcp_servers_persists_and_writes_codex_config() {
    let dir = std::env::temp_dir().join(format!("rixlcode-mcp-save-{}", std::process::id()));
    unsafe {
        std::env::set_var("HOME", &dir);
        std::env::set_var("CODEX_HOME", dir.join("codex"));
    }
    crate::mcp_config::save_servers(&[stdio("fs", "mcp-fs")]);
    let loaded = crate::persist::load_settings();
    assert_eq!(loaded.mcp_servers.len(), 1);
    assert_eq!(loaded.mcp_servers[0].name, "fs");
    let toml = std::fs::read_to_string(dir.join("codex/config.toml")).unwrap();
    assert!(toml.contains("[mcp_servers.fs]"), "{toml}");

    // Removing the last server strips the block from config.toml.
    crate::mcp_config::save_servers(&[]);
    let toml = std::fs::read_to_string(dir.join("codex/config.toml")).unwrap();
    assert!(!toml.contains("mcp_servers"), "{toml}");
    assert!(crate::persist::load_settings().mcp_servers.is_empty());
}

#[test]
fn acp_mcp_servers_maps_transports_and_skips_disabled() {
    let mut http = stdio("web", "https://mcp.example.com");
    http.transport = McpTransport::Http;
    http.env.insert("X-Key".into(), "k".into());
    let mut off = stdio("off", "off-bin");
    off.enabled = false;
    let out = acp_mcp_servers(&[stdio("fs", "mcp-fs --verbose"), http, off]);
    assert_eq!(
        out,
        vec![
            json!({"command": "mcp-fs", "args": ["--verbose"], "env": []}),
            json!({"type": "http", "url": "https://mcp.example.com", "headers": [{"name": "X-Key", "value": "k"}]}),
        ]
    );
}

#[test]
fn parse_status_page_reads_servers_and_cursor() {
    let result = json!({
        "data": [
            {"name": "fs", "runtimeStatus": "connected", "authStatus": "unsupported",
             "tools": {"a": {}, "b": {}}, "resources": [], "resourceTemplates": []},
            {"name": "web", "runtimeStatus": "failed", "authStatus": "notLoggedIn",
             "tools": {}, "toolsError": "spawn failed", "resources": [], "resourceTemplates": []},
        ],
        "nextCursor": "p2",
    });
    let (statuses, cursor) = parse_status_page(&result);
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].name, "fs");
    assert_eq!(statuses[0].runtime, "connected");
    assert_eq!(statuses[0].tools, 2);
    assert_eq!(statuses[1].runtime, "failed");
    assert_eq!(statuses[1].tools_error.as_deref(), Some("spawn failed"));
    assert_eq!(cursor, Some(json!("p2")));

    let (_, none) = parse_status_page(&json!({"data": [], "nextCursor": null}));
    assert!(none.is_none());
}

#[test]
fn read_mcp_status_handshakes_and_paginates() {
    let server_out = concat!(
        r#"{"id":1,"result":{}}"#,
        "\n",
        r#"{"id":2,"result":{"data":[{"name":"fs","runtimeStatus":"connected","authStatus":"unsupported","tools":{"t":{}},"resources":[],"resourceTemplates":[]}],"nextCursor":"c1"}}"#,
        "\n",
        r#"{"id":3,"result":{"data":[{"name":"web","runtimeStatus":"failed","authStatus":"unknown","tools":{},"resources":[],"resourceTemplates":[]}]}}"#,
        "\n",
    );
    let mut stdin = Vec::new();
    let statuses = crate::backend::read_mcp_status(&mut stdin, server_out.as_bytes()).unwrap();
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].name, "fs");
    assert_eq!(statuses[1].name, "web");

    // The client sent initialize + two paginated list requests.
    let sent: Vec<serde_json::Value> = String::from_utf8(stdin).unwrap().lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    assert_eq!(sent[0]["method"], json!("initialize"));
    assert_eq!(sent[2]["method"], json!("mcpServerStatus/list"));
    assert_eq!(sent[3]["params"]["cursor"], json!("c1"));
}

#[test]
fn read_mcp_status_eof_errors() {
    let mut stdin = Vec::new();
    // Init reply only — stdout closes before any status page.
    let out = r#"{"id":1,"result":{}}"#;
    assert!(crate::backend::read_mcp_status(&mut stdin, out.as_bytes()).is_err());
}
