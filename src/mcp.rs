//! MCP server configuration: the `McpServer` model persisted in
//! settings.json, the ACP `session/new` `mcpServers` mapping, and the
//! `mcpServerStatus/list` parser behind the settings status dots. The
//! `~/.codex/config.toml` `[mcp_servers]` writer/reader lives in
//! `mcp_config.rs`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// How the agent reaches the server — a stdio subprocess or a
/// streamable-http endpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    /// Spawn `command` and speak MCP over its stdio.
    #[default]
    Stdio,
    /// Streamable HTTP endpoint — `command` carries the URL.
    Http,
}

/// One configured MCP server. `command` is the spawn command line for
/// stdio servers and the endpoint URL for http ones; `env` maps to `env`
/// (stdio) / `http_headers` (http) in codex's config and to ACP's
/// `env`/`headers` lists.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct McpServer {
    pub name: String,
    pub transport: McpTransport,
    pub command: String,
    pub env: BTreeMap<String, String>,
    /// Enabled unless explicitly disabled — matches codex's `enabled`
    /// default.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for McpServer {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: McpTransport::Stdio,
            command: String::new(),
            env: BTreeMap::new(),
            enabled: true,
        }
    }
}

/// Live status for one server from codex `mcpServerStatus/list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpStatus {
    pub name: String,
    /// `McpServerConnectionStatus` — connected/starting/failed/…; empty
    /// when the server reported none.
    pub runtime: String,
    /// `McpAuthStatus` — unsupported/notLoggedIn/oAuth/…
    pub auth: String,
    pub tools: usize,
    pub tools_error: Option<String>,
}

/// Parse the env/headers input: `KEY=VALUE` pairs separated by commas or
/// newlines. Entries without `=` are ignored.
pub(crate) fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.split([',', '\n'])
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Split a command line into argv — whitespace-separated with double
/// quotes grouping (`npx "-y @scope/pkg"`). Not a shell: no escapes or
/// expansion.
pub(crate) fn split_command(line: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.trim().chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    argv.push(std::mem::take(&mut cur));
                }
            },
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        argv.push(cur);
    }
    argv
}

/// `session/new`'s `mcpServers` param — ACP's tagged union: stdio entries
/// carry `{command, args, env:[{name,value}]}`, http entries
/// `{type:"http", url, headers:[…]}`. Disabled servers are omitted — the
/// agent should never see them.
pub(crate) fn acp_mcp_servers(servers: &[McpServer]) -> Vec<Value> {
    let env_list = |env: &BTreeMap<String, String>| Value::Array(env.iter().map(|(k, v)| json!({"name": k, "value": v})).collect());
    servers
        .iter()
        .filter(|s| s.enabled)
        .map(|s| match s.transport {
            McpTransport::Stdio => {
                let argv = split_command(&s.command);
                let (cmd, args) = argv.split_first().map_or((String::new(), &[][..]), |(c, a)| (c.clone(), a));
                json!({"command": cmd, "args": args, "env": env_list(&s.env)})
            },
            McpTransport::Http => json!({"type": "http", "url": s.command, "headers": env_list(&s.env)}),
        })
        .collect()
}

/// Parse one `mcpServerStatus/list` result into `(statuses, next_cursor)`.
pub(crate) fn parse_status_page(result: &Value) -> (Vec<McpStatus>, Option<Value>) {
    let statuses = result["data"]
        .as_array()
        .map(|data| {
            data.iter()
                .map(|s| McpStatus {
                    name: s["name"].as_str().unwrap_or_default().to_string(),
                    runtime: s["runtimeStatus"].as_str().unwrap_or_default().to_string(),
                    auth: s["authStatus"].as_str().unwrap_or_default().to_string(),
                    tools: s["tools"].as_object().map_or(0, |t| t.len()),
                    tools_error: s["toolsError"].as_str().map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    (statuses, result.get("nextCursor").filter(|c| !c.is_null()).cloned())
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod mcp_tests;
