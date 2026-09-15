//! `codex app-server` wire format: request builders and replies to
//! server-initiated requests. Pure `serde_json::Value` construction —
//! no I/O, so every shape is unit-testable.

use serde_json::{Value, json};

/// `initialize` request — clientInfo identifies us in the server's logs.
pub(crate) fn initialize_req(id: i64) -> Value {
    json!({
        "method": "initialize",
        "id": id,
        "params": {
            "clientInfo": {"name": "rixlcode", "title": "Rixl Code", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": false, "requestAttestation": false},
        },
    })
}

/// `thread/start`: one ephemeral thread per turn (no history is kept, so a
/// fresh thread per send matches the old `codex exec` behavior). `sandbox`
/// is a `SandboxMode` string; approvals are off — there's no approval UI.
pub(crate) fn thread_start_req(id: i64, model: &str, sandbox: &str) -> Value {
    let mut params = json!({
        "approvalPolicy": "never",
        "sandbox": sandbox,
        "ephemeral": true,
        "cwd": std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into()),
    });
    if model != "default" {
        params["model"] = json!(model);
    }
    json!({"method": "thread/start", "id": id, "params": params})
}

/// `turn/start`: the user's prompt as a single text input.
pub(crate) fn turn_start_req(id: i64, thread_id: &str, prompt: &str) -> Value {
    json!({
        "method": "turn/start",
        "id": id,
        "params": {
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt, "text_elements": []}],
        },
    })
}

/// `model/list` — one page of the provider's catalog. `cursor` is the
/// previous page's `nextCursor`; `None` requests the first page.
pub(crate) fn model_list_req(id: i64, cursor: Option<&Value>) -> Value {
    let mut params = json!({"includeHidden": false, "limit": 100});
    if let Some(c) = cursor {
        params["cursor"] = c.clone();
    }
    json!({"method": "model/list", "id": id, "params": params})
}

/// Parse one `model/list` result into `(models, next_cursor)`. Hidden
/// entries are dropped — the picker never offers them.
pub(crate) fn parse_model_page(result: &Value) -> (Vec<crate::model::ModelInfo>, Option<Value>) {
    let models = result["data"]
        .as_array()
        .map(|data| {
            data.iter()
                .filter(|m| !m["hidden"].as_bool().unwrap_or(false))
                .filter_map(|m| {
                    let id = m["id"].as_str()?;
                    if id.is_empty() {
                        return None;
                    }
                    Some(crate::model::ModelInfo {
                        id: id.into(),
                        label: m["displayName"].as_str().unwrap_or(id).into(),
                        description: m["description"].as_str().unwrap_or("").into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (models, result["nextCursor"].as_str().map(|s| json!(s)))
}

/// JSON-RPC response for a server-initiated request. There's no approval
/// UI, so approvals are declined and everything else gets a generic error —
/// matching `codex exec`'s non-interactive behavior.
pub(crate) fn request_reply(method: &str, msg: &Value) -> Value {
    let id = msg["id"].clone();
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({"decision": "decline"}),
        "applyPatchApproval" | "execCommandApproval" => json!({"decision": "denied"}),
        "mcpServer/elicitation/request" => json!({"action": "decline", "content": null, "_meta": null}),
        _ => {
            return json!({"id": id, "error": {"code": -32603, "message": format!("rixlcode cannot answer {method}")}});
        },
    };
    json!({"id": id, "result": result})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_start_maps_sandbox_and_approval() {
        let req = thread_start_req(2, "default", "workspace-write");
        assert_eq!(req["method"], json!("thread/start"));
        assert_eq!(req["params"]["sandbox"], json!("workspace-write"));
        // No approval UI exists — the server must never block on one.
        assert_eq!(req["params"]["approvalPolicy"], json!("never"));
        assert_eq!(req["params"]["ephemeral"], json!(true));
    }

    #[test]
    fn model_only_set_when_not_default() {
        assert!(thread_start_req(2, "default", "read-only")["params"].get("model").is_none());
        assert_eq!(thread_start_req(2, "gpt-5", "read-only")["params"]["model"], json!("gpt-5"));
    }

    #[test]
    fn turn_start_wraps_prompt_as_text_input() {
        let req = turn_start_req(3, "tid", "hello");
        assert_eq!(req["params"]["threadId"], json!("tid"));
        assert_eq!(req["params"]["input"][0]["type"], json!("text"));
        assert_eq!(req["params"]["input"][0]["text"], json!("hello"));
    }

    #[test]
    fn initialize_advertises_no_experimental_api() {
        let req = initialize_req(1);
        assert_eq!(req["method"], json!("initialize"));
        assert_eq!(req["params"]["capabilities"]["experimentalApi"], json!(false));
    }

    #[test]
    fn model_list_omits_cursor_on_first_page() {
        let req = model_list_req(2, None);
        assert_eq!(req["method"], json!("model/list"));
        assert_eq!(req["params"]["includeHidden"], json!(false));
        assert!(req["params"].get("cursor").is_none());
        let paged = model_list_req(3, Some(&json!("abc")));
        assert_eq!(paged["params"]["cursor"], json!("abc"));
    }

    #[test]
    fn parse_model_page_drops_hidden_and_reads_cursor() {
        let result = json!({
            "data": [
                {"id": "gpt-6", "displayName": "GPT-6", "description": "capable", "hidden": false},
                {"id": "gpt-6-mini", "hidden": true},
                {"id": "gpt-5", "hidden": false},
                {"id": "", "hidden": false},
            ],
            "nextCursor": "page-2",
        });
        let (models, cursor) = parse_model_page(&result);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_ref()).collect();
        assert_eq!(ids, ["gpt-6", "gpt-5"]);
        assert_eq!(models[0].label.as_ref(), "GPT-6");
        assert_eq!(models[0].description.as_ref(), "capable");
        // Missing displayName falls back to the id.
        assert_eq!(models[1].label.as_ref(), "gpt-5");
        assert_eq!(cursor, Some(json!("page-2")));
    }

    #[test]
    fn parse_model_page_last_page_has_no_cursor() {
        let (models, cursor) = parse_model_page(&json!({"data": [], "nextCursor": null}));
        assert!(models.is_empty());
        assert_eq!(cursor, None);
    }
}
