//! MCP (Model Context Protocol) wire format: JSON-RPC 2.0 request
//! builders, the `tools/list`/`prompts/list` catalog, and the mapping
//! from a picker model id onto `tools/call`/`prompts/get` arguments.
//! Pure `serde_json::Value` construction — no process I/O, so every
//! shape is unit-testable.

use std::io::Write;

use serde_json::{Value, json};

use crate::model::ModelInfo;

/// Protocol revision this client speaks.
pub(super) const PROTOCOL_VERSION: &str = "2025-06-18";

/// Write one JSON-RPC message as a single NDJSON line.
pub(super) fn send(stdin: &mut dyn Write, v: &Value) -> Result<(), String> {
    writeln!(stdin, "{v}").map_err(|e| format!("mcp stdin: {e}"))
}

/// `initialize` — empty `capabilities`: the server can't ask us for
/// sampling, roots, or elicitation, so it never blocks on a reply we
/// can't give.
pub(super) fn initialize_req(id: i64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "rixlcode", "version": env!("CARGO_PKG_VERSION")},
        }
    })
}

/// `notifications/initialized` — completes the handshake.
pub(super) fn initialized_note() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// `tools/list` / `prompts/list` — `cursor` continues a paginated listing.
pub(super) fn list_req(id: i64, method: &str, cursor: Option<&str>) -> Value {
    let params = cursor.map_or_else(|| json!({}), |c| json!({"cursor": c}));
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// `tools/call` — `args` comes from `Listings::tool_args`.
pub(super) fn call_req(id: i64, name: &str, args: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// `prompts/get` — `args` comes from `Listings::prompt_args`.
pub(super) fn get_req(id: i64, name: &str, args: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "prompts/get", "params": {"name": name, "arguments": args}})
}

/// Reply to a server-initiated request: `ping` answers `{}`, everything
/// else gets method-not-found — we advertise no client capabilities, so
/// a server that still asks (sampling, roots, elicitation) can't hang
/// waiting on us.
pub(super) fn server_reply(msg: &Value) -> Value {
    let id = msg["id"].clone();
    match msg["method"].as_str() {
        Some("ping") => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
    }
}

/// What the selected pseudo-model resolves to against a fresh listing.
pub(super) enum Target {
    /// `tools/call` this tool with `args`.
    Tool { name: String, args: Value },
    /// `prompts/get` this template with `args`.
    Prompt { name: String, args: Value },
    /// The server-name pseudo-model — the listing itself is the reply.
    Server,
}

/// The server's capabilities snapshot: `initialize`'s serverInfo plus the
/// raw `tools/list`/`prompts/list` entries across all pages.
pub(super) struct Listings {
    /// `serverInfo.name` — the fallback pseudo-model's id and label.
    pub server: String,
    /// `serverInfo.version` — shown in the server-info reply.
    pub version: String,
    /// Raw tool entries (`{name, title?, description?, inputSchema}`).
    pub tools: Vec<Value>,
    /// Raw prompt entries (`{name, title?, description?, arguments?}`).
    pub prompts: Vec<Value>,
}

impl Listings {
    /// The picker catalog: one pseudo-model per tool (`tool:<name>`) and
    /// per prompt (`prompt:<name>`). A server with neither gets a single
    /// entry named after it — selecting it prints the server's info.
    pub(super) fn models(&self) -> Vec<ModelInfo> {
        let entry = |prefix: &str, v: &Value, fallback: &str| ModelInfo {
            id: format!("{prefix}:{}", v["name"].as_str().unwrap_or_default()).into(),
            label: v["title"].as_str().or_else(|| v["name"].as_str()).unwrap_or_default().into(),
            description: v["description"].as_str().unwrap_or(fallback).into(),
            ..Default::default()
        };
        let mut out: Vec<ModelInfo> = self
            .tools
            .iter()
            .filter(|t| t["name"].as_str().is_some_and(|n| !n.is_empty()))
            .map(|t| entry("tool", t, "tool"))
            .collect();
        out.extend(
            self.prompts
                .iter()
                .filter(|p| p["name"].as_str().is_some_and(|n| !n.is_empty()))
                .map(|p| entry("prompt", p, "prompt template")),
        );
        if out.is_empty() {
            out.push(ModelInfo {
                id: self.server.clone().into(),
                label: self.server.clone().into(),
                description: "server info — no tools or prompts".into(),
                ..Default::default()
            });
        }
        out
    }

    /// Resolve the picker's model id: `tool:`/`prompt:` prefixes are
    /// authoritative; a bare id (hand-edited settings, a stale cache)
    /// resolves as a tool first, then a prompt, then the server-info
    /// pseudo-model.
    pub(super) fn resolve(&self, model: &str, text: &str) -> Target {
        match model.split_once(':') {
            Some(("tool", name)) => return Target::Tool { name: name.to_string(), args: self.tool_args(name, text) },
            Some(("prompt", name)) => return Target::Prompt { name: name.to_string(), args: self.prompt_args(name, text) },
            _ => {},
        }
        if self.tool(model).is_some() {
            Target::Tool { name: model.to_string(), args: self.tool_args(model, text) }
        } else if self.prompt(model).is_some() {
            Target::Prompt { name: model.to_string(), args: self.prompt_args(model, text) }
        } else {
            Target::Server
        }
    }

    fn tool(&self, name: &str) -> Option<&Value> {
        self.tools.iter().find(|t| t["name"].as_str() == Some(name))
    }

    fn prompt(&self, name: &str) -> Option<&Value> {
        self.prompts.iter().find(|p| p["name"].as_str() == Some(name))
    }

    /// `tools/call` arguments: the user's text lands on the tool's first
    /// *required* string property, else its first string property, else a
    /// conventional `prompt` key. Unknown schemas get `{"prompt": text}` —
    /// a tool that can't use it rejects the call with a clean RPC error.
    fn tool_args(&self, name: &str, text: &str) -> Value {
        let schema = self.tool(name).map(|t| &t["inputSchema"]);
        let props = schema.and_then(|s| s["properties"].as_object());
        let required = schema.and_then(|s| s["required"].as_array());
        let is_string = |p: Option<&Value>| p.and_then(|s| s["type"].as_str()) == Some("string");
        let key = props.and_then(|p| {
            required
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_str())
                .find(|r| is_string(p.get(*r)))
                .map(str::to_string)
                .or_else(|| p.iter().find(|(_, s)| is_string(Some(s))).map(|(k, _)| k.clone()))
        });
        json!({ key.unwrap_or_else(|| "prompt".to_string()): text })
    }

    /// `prompts/get` arguments: the text lands on the template's first
    /// required argument, else its first argument. A template that
    /// declares no arguments takes `{}` — there's no slot for the text.
    fn prompt_args(&self, name: &str, text: &str) -> Value {
        let args = self.prompt(name).and_then(|p| p["arguments"].as_array());
        match args {
            Some(list) if !list.is_empty() => {
                let key = list
                    .iter()
                    .find(|a| a["required"].as_bool() == Some(true))
                    .or_else(|| list.first())
                    .and_then(|a| a["name"].as_str())
                    .unwrap_or("prompt");
                json!({ key: text })
            },
            _ => json!({}),
        }
    }
}
