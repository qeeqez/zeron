//! MCP result decoding: `tools/call` content blocks, `prompts/get`
//! messages, and server notifications → `AgentEvent`s. Pure mapping —
//! the pump owns the channel.

use serde_json::Value;

use super::AgentEvent;

/// One MCP content block → events. Text becomes `TextDelta` (the reply
/// bubble); image/audio/resource blocks can't render as text, so they
/// land on the tool card as a one-line summary.
fn content_block(block: &Value, ix: usize, out: &mut Vec<AgentEvent>) {
    match block["type"].as_str() {
        Some("text") => {
            if let Some(t) = block["text"].as_str().filter(|t| !t.is_empty()) {
                out.push(AgentEvent::TextDelta(t.to_string().into()));
            }
        },
        Some(other) => {
            let summary = match other {
                "image" | "audio" => format!("[{other}: {}]", block["mimeType"].as_str().unwrap_or("?")),
                "resource" => format!("[resource: {}]", block["resource"]["uri"].as_str().unwrap_or("?")),
                "resource_link" => format!("[resource: {}]", block["uri"].as_str().unwrap_or("?")),
                _ => format!("[{other}]"),
            };
            out.push(AgentEvent::ToolCallDelta { ix, output: summary.into() });
        },
        None => {},
    }
}

/// A `tools/call` result → events for the card at `ix`. `isError` flips
/// the card to failed; the error text still streams as the reply.
pub(super) fn call_result(result: &Value, ix: usize) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    if let Some(blocks) = result["content"].as_array() {
        for b in blocks {
            content_block(b, ix, &mut out);
        }
    }
    out.push(AgentEvent::ToolCallEnd { ix, ok: result["isError"].as_bool() != Some(true) });
    out
}

/// A `prompts/get` result → events: each message's text becomes its own
/// bubble so a multi-message template doesn't merge into one blob.
pub(super) fn prompt_result(result: &Value) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    for msg in result["messages"].as_array().into_iter().flatten() {
        if let Some(t) = msg["content"]["text"].as_str().filter(|t| !t.is_empty()) {
            out.push(AgentEvent::TextStart);
            out.push(AgentEvent::TextDelta(t.to_string().into()));
        }
    }
    out
}

/// The server-info pseudo-model's reply — what the server calls itself.
pub(super) fn server_info_text(server: &str, version: &str) -> String {
    match version.is_empty() {
        true => format!("{server} — no tools or prompts exposed."),
        false => format!("{server} {version} — no tools or prompts exposed."),
    }
}

/// A server notification while a tool call is live → card output.
/// `notifications/progress` carries `progress`/`total`/`message`;
/// `notifications/message` is the server's log stream. Everything else
/// (cancelled, list-changed) doesn't map — `None`.
pub(super) fn notification(msg: &Value, ix: usize) -> Option<AgentEvent> {
    let params = &msg["params"];
    let line = match msg["method"].as_str()? {
        "notifications/progress" => {
            let progress = params["progress"].as_f64().unwrap_or_default();
            let total = params["total"].as_f64();
            let note = params["message"].as_str().unwrap_or_default();
            match total {
                Some(t) if t > 0.0 => format!("{progress:.0}/{t:.0} {note}").trim().to_string(),
                _ if !note.is_empty() => note.to_string(),
                _ => format!("{progress:.0}"),
            }
        },
        "notifications/message" => {
            let level = params["level"].as_str().unwrap_or("info");
            format!("[{level}] {}", params["data"].as_str().unwrap_or_default())
        },
        _ => return None,
    };
    Some(AgentEvent::ToolCallDelta { ix, output: format!("{line}\n").into() })
}
