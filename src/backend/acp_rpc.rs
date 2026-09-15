//! ACP (Agent Client Protocol) wire format: JSON-RPC 2.0 request builders
//! and replies to agent-initiated requests. Pure `serde_json::Value`
//! construction plus the two `fs/*` file handlers — no process I/O, so
//! every shape is unit-testable.

use std::io::Write;

use serde_json::{Value, json};

use super::AccessMode;
use crate::model::ModelInfo;

/// Write one JSON-RPC message as a single NDJSON line.
pub(super) fn send(stdin: &mut dyn Write, v: &Value) -> Result<(), String> {
    writeln!(stdin, "{v}").map_err(|e| format!("acp stdin: {e}"))
}

/// `initialize` — protocol v1. `write_fs` advertises `fs/write_text_file`
/// (Agent mode with write access only); reads are always offered.
pub(super) fn initialize_req(id: i64, write_fs: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": id,
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": {"readTextFile": true, "writeTextFile": write_fs},
                "terminal": false,
            },
            "clientInfo": {"name": "rixlcode", "title": "Rixl Code", "version": env!("CARGO_PKG_VERSION")},
        },
    })
}

/// `session/new` — one session per turn (no history is kept, matching the
/// codex backend's ephemeral threads). No MCP servers are configured.
pub(super) fn session_new_req(id: i64, cwd: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/new",
        "id": id,
        "params": {"cwd": cwd, "mcpServers": []},
    })
}

/// `session/prompt` — the user's prompt as a single text block.
pub(super) fn prompt_req(id: i64, session_id: &str, prompt: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/prompt",
        "id": id,
        "params": {"sessionId": session_id, "prompt": [{"type": "text", "text": prompt}]},
    })
}

/// `session/set_mode` — pick one of the modes `session/new` advertised.
pub(super) fn set_mode_req(id: i64, session_id: &str, mode_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/set_mode",
        "id": id,
        "params": {"sessionId": session_id, "modeId": mode_id},
    })
}

/// `session/set_config_option` — select a model when the agent exposes a
/// `category: "model"` select in `configOptions`.
pub(super) fn set_config_req(id: i64, session_id: &str, config_id: &str, value: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/set_config_option",
        "id": id,
        "params": {"sessionId": session_id, "configId": config_id, "value": value},
    })
}

/// `session/set_model` — legacy model selection for agents that report
/// `models.availableModels` instead of a config option.
pub(super) fn set_model_req(id: i64, session_id: &str, model_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/set_model",
        "id": id,
        "params": {"sessionId": session_id, "modelId": model_id},
    })
}

/// What the client may do on the agent's behalf — derived from the chat
/// mode and access setting at spawn time.
pub(super) struct Policy {
    /// Auto-approve `session/request_permission` (Agent mode + write
    /// access — the codex backend's `approvalPolicy: "never"` equivalent).
    pub auto_allow: bool,
    /// Honor `fs/write_text_file` requests (same condition as `auto_allow`).
    pub write_fs: bool,
    /// Confine `fs/write_text_file` to the working directory
    /// (workspace-write access).
    pub workspace_only: bool,
    /// Working directory for the workspace-only check.
    pub cwd: std::path::PathBuf,
}

impl Policy {
    pub(super) fn of(mode: &str, access: AccessMode, cwd: std::path::PathBuf) -> Self {
        let write = mode == "Agent" && access != AccessMode::ReadOnly;
        Self {
            auto_allow: write,
            write_fs: write,
            workspace_only: access == AccessMode::WorkspaceWrite,
            cwd,
        }
    }
}

/// JSON-RPC response for an agent-initiated request. Permission prompts
/// follow `policy` (no approval UI exists); `fs/*` does real file I/O;
/// everything else gets method-not-found so the agent can't hang waiting.
pub(super) fn request_reply(method: &str, msg: &Value, policy: &Policy) -> Value {
    let id = msg["id"].clone();
    match method {
        "session/request_permission" => permission_reply(id, &msg["params"], policy),
        "fs/read_text_file" => read_file_reply(id, &msg["params"]),
        "fs/write_text_file" => write_file_reply(id, &msg["params"], policy),
        _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("rixlcode cannot answer {method}")}}),
    }
}

/// Pick a permission option without a UI: `allow_once` when the turn may
/// write, otherwise `reject_once`. `cancelled` is the fallback when the
/// agent offered no option of the right kind.
fn permission_reply(id: Value, params: &Value, policy: &Policy) -> Value {
    let options = params["options"].as_array();
    let pick = |kind: &str| {
        options
            .and_then(|os| os.iter().find(|o| o["kind"].as_str() == Some(kind)))
            .and_then(|o| o["optionId"].as_str())
    };
    let selected = if policy.auto_allow {
        pick("allow_once").or_else(|| pick("allow_always"))
    } else {
        pick("reject_once").or_else(|| pick("reject_always"))
    };
    let outcome = match selected {
        Some(option_id) => json!({"outcome": "selected", "optionId": option_id}),
        None => json!({"outcome": "cancelled"}),
    };
    json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})
}

/// `fs/read_text_file`: read `path`, honoring the optional 1-based `line`
/// window and `limit`. Errors go back as JSON-RPC errors, not panics.
fn read_file_reply(id: Value, params: &Value) -> Value {
    let Some(path) = params["path"].as_str() else {
        return err_reply(id, -32602, "fs/read_text_file: missing path");
    };
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let content = match (params["line"].as_u64(), params["limit"].as_u64()) {
                (None, None) => text,
                (line, limit) => {
                    let skip = line.unwrap_or(1).saturating_sub(1) as usize;
                    let iter = text.lines().skip(skip);
                    match limit {
                        Some(n) => iter.take(n as usize).collect::<Vec<_>>().join("\n"),
                        None => iter.collect::<Vec<_>>().join("\n"),
                    }
                },
            };
            json!({"jsonrpc": "2.0", "id": id, "result": {"content": content}})
        },
        Err(e) => err_reply(id, -32603, format!("fs/read_text_file {path}: {e}")),
    }
}

/// `fs/write_text_file`: only when the turn advertised write access, and
/// confined to the workspace under workspace-write.
fn write_file_reply(id: Value, params: &Value, policy: &Policy) -> Value {
    if !policy.write_fs {
        return err_reply(id, -32603, "fs/write_text_file: writes not permitted in this mode");
    }
    let (Some(path), Some(content)) = (params["path"].as_str(), params["content"].as_str()) else {
        return err_reply(id, -32602, "fs/write_text_file: missing path or content");
    };
    let path = std::path::Path::new(path);
    if policy.workspace_only && !path.starts_with(&policy.cwd) {
        return err_reply(id, -32603, "fs/write_text_file: path outside the workspace");
    }
    match std::fs::write(path, content) {
        Ok(()) => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        Err(e) => err_reply(id, -32603, format!("fs/write_text_file {}: {e}", path.display())),
    }
}

fn err_reply(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Refresh the shared catalog from `session/new`: `configOptions` with
/// `category: "model"` first, then the legacy `models.availableModels`.
/// Persisted to the model cache so the picker sees it next launch.
pub(super) fn cache_models(models: &parking_lot::Mutex<Vec<ModelInfo>>, result: &Value) {
    let mut found = config_models(result);
    if found.is_empty() {
        found = legacy_models(result);
    }
    if !found.is_empty() {
        *models.lock() = found.clone();
        // Unit tests must not touch ~/.rixl — skip the disk write there.
        #[cfg(not(test))]
        crate::persist::save_model_cache("acp", &found);
    }
}

/// `configOptions` entries categorized as model selectors — options may
/// be a flat list or grouped under headers.
fn config_models(result: &Value) -> Vec<ModelInfo> {
    let Some(opt) = model_option(result) else { return vec![] };
    let mut out = vec![];
    for v in opt["options"].as_array().into_iter().flatten() {
        if v.get("value").is_some() {
            push_model(&mut out, &v["value"], &v["name"]);
        }
        for sub in v["options"].as_array().into_iter().flatten() {
            push_model(&mut out, &sub["value"], &sub["name"]);
        }
    }
    out
}

/// Legacy `models.availableModels`: `{modelId, name}` entries.
fn legacy_models(result: &Value) -> Vec<ModelInfo> {
    let mut out = vec![];
    for m in result["models"]["availableModels"].as_array().into_iter().flatten() {
        push_model(&mut out, &m["modelId"], &m["name"]);
    }
    out
}

fn push_model(out: &mut Vec<ModelInfo>, id: &Value, name: &Value) {
    if let Some(id) = id.as_str() {
        out.push(ModelInfo {
            id: id.into(),
            label: name.as_str().unwrap_or(id).into(),
            description: "".into(),
        });
    }
}

/// The `configOptions` entry that selects a model, if the agent has one.
fn model_option(result: &Value) -> Option<&Value> {
    result["configOptions"].as_array()?.iter().find(|o| o["category"].as_str() == Some("model"))
}

/// Build the model-selection request for this turn, or `None` when the
/// picker is on "default" or the agent doesn't offer the model.
pub(super) fn model_request(id: i64, sid: &str, model: &str, result: &Value) -> Option<Value> {
    if model == "default" {
        return None;
    }
    if let Some(opt) = model_option(result) {
        let flat = opt["options"].as_array().into_iter().flatten();
        let grouped = flat.clone().flat_map(|g| g["options"].as_array().into_iter().flatten());
        if flat.chain(grouped).any(|v| v["value"].as_str() == Some(model)) {
            return opt["id"].as_str().map(|cid| set_config_req(id, sid, cid, model));
        }
        return None;
    }
    let known = result["models"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|m| m["modelId"].as_str() == Some(model));
    known.then(|| set_model_req(id, sid, model))
}

/// Map the chat mode onto a `session/new` mode id. "Agent" stays on the
/// agent's default unless it literally offers an "agent" mode; "Ask"
/// falls back to a plan-ish mode since both are read-only.
pub(super) fn mode_pick(mode: &str, result: &Value) -> Option<String> {
    let modes = result["modes"]["availableModes"].as_array()?;
    let find = |needle: &str| {
        modes.iter().find_map(|m| {
            let id = m["id"].as_str().unwrap_or("");
            let name = m["name"].as_str().unwrap_or("");
            (id.eq_ignore_ascii_case(needle) || name.to_lowercase().contains(needle)).then(|| id.to_string())
        })
    };
    match mode {
        "Plan" => find("plan"),
        "Ask" => find("ask").or_else(|| find("plan")),
        _ => find("agent"),
    }
}
