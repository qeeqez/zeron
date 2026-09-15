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
/// is a `SandboxMode` string, `approval` the `AskForApproval` policy, and
/// `cwd` the thread's working directory (project root or its worktree).
pub(crate) fn thread_start_req(id: i64, model: &str, sandbox: &str, approval: &str, cwd: &std::path::Path) -> Value {
    let mut params = json!({
        "approvalPolicy": approval,
        "sandbox": sandbox,
        "ephemeral": true,
        "cwd": cwd.to_string_lossy(),
    });
    params["model"] = json!(model);
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

/// `thread/list` — one page of past threads, newest first. `cursor` is the
/// previous page's `nextCursor`; `None` requests the first page.
pub(crate) fn thread_list_req(id: i64, cursor: Option<&Value>) -> Value {
    let mut params = json!({"limit": 25});
    if let Some(c) = cursor {
        params["cursor"] = c.clone();
    }
    json!({"method": "thread/list", "id": id, "params": params})
}

/// `thread/resume` — reopen a past thread. `model`/`sandbox`/`approval`
/// override the thread's stored settings so a continued turn honors the
/// chat's current configuration; pass `None`s for a read-only fetch.
pub(crate) fn thread_resume_req(id: i64, thread_id: &str, model: Option<&str>, sandbox: Option<&str>, approval: Option<&str>) -> Value {
    let mut params = json!({"threadId": thread_id});
    if let Some(m) = model {
        params["model"] = json!(m);
    }
    if let Some(s) = sandbox {
        params["sandbox"] = json!(s);
    }
    if let Some(a) = approval {
        params["approvalPolicy"] = json!(a);
    }
    json!({"method": "thread/resume", "id": id, "params": params})
}

/// Parse one `thread/list` result into `(sessions, next_cursor)`. Ephemeral
/// threads (our own per-turn threads) are dropped — they hold no history
/// worth reopening.
pub(crate) fn parse_thread_page(result: &Value) -> (Vec<super::SessionInfo>, Option<Value>) {
    let sessions = result["data"]
        .as_array()
        .map(|data| {
            data.iter()
                .filter(|t| !t["ephemeral"].as_bool().unwrap_or(false))
                .filter_map(|t| {
                    let id = t["id"].as_str()?;
                    if id.is_empty() {
                        return None;
                    }
                    Some(super::SessionInfo {
                        id: id.to_string(),
                        title: thread_title(t),
                        updated: t["updatedAt"].as_u64().unwrap_or(0),
                        cwd: t["cwd"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (sessions, result["nextCursor"].as_str().map(|s| json!(s)))
}

/// Display title for a thread: its `name`, else the first non-empty
/// preview line, else a placeholder.
pub(crate) fn thread_title(thread: &Value) -> String {
    if let Some(name) = thread["name"].as_str().filter(|n| !n.is_empty()) {
        return name.to_string();
    }
    thread["preview"]
        .as_str()
        .and_then(|p| p.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("Untitled thread")
        .to_string()
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
        let cwd = std::path::Path::new("/tmp/thread-wt");
        let req = thread_start_req(2, "gpt-5", "workspace-write", "on-failure", cwd);
        assert_eq!(req["method"], json!("thread/start"));
        assert_eq!(req["params"]["sandbox"], json!("workspace-write"));
        assert_eq!(req["params"]["approvalPolicy"], json!("on-failure"));
        assert_eq!(req["params"]["ephemeral"], json!(true));
        // The thread's working directory goes on the wire — a worktree
        // thread's server must see the worktree, not the process cwd.
        assert_eq!(req["params"]["cwd"], json!("/tmp/thread-wt"));
    }

    #[test]
    fn thread_start_always_sends_the_model() {
        // No synthetic "default" — the concrete id always goes on the wire.
        let cwd = std::path::Path::new("/tmp");
        assert_eq!(thread_start_req(2, "gpt-5", "read-only", "on-request", cwd)["params"]["model"], json!("gpt-5"));
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

    #[test]
    fn thread_list_omits_cursor_on_first_page() {
        let req = thread_list_req(2, None);
        assert_eq!(req["method"], json!("thread/list"));
        assert!(req["params"].get("cursor").is_none());
        let paged = thread_list_req(3, Some(&json!("2026-09-11T17:04:11Z")));
        assert_eq!(paged["params"]["cursor"], json!("2026-09-11T17:04:11Z"));
    }

    #[test]
    fn thread_resume_sends_id_and_overrides() {
        let req = thread_resume_req(2, "tid-1", Some("gpt-5"), Some("workspace-write"), Some("never"));
        assert_eq!(req["method"], json!("thread/resume"));
        assert_eq!(req["params"]["threadId"], json!("tid-1"));
        assert_eq!(req["params"]["model"], json!("gpt-5"));
        assert_eq!(req["params"]["sandbox"], json!("workspace-write"));
        assert_eq!(req["params"]["approvalPolicy"], json!("never"));
        // A bare resume (history fetch) carries only the thread id.
        let bare = thread_resume_req(3, "tid-1", None, None, None);
        assert_eq!(bare["params"], json!({"threadId": "tid-1"}));
    }

    #[test]
    fn parse_thread_page_reads_sessions_and_cursor() {
        let result = json!({
            "data": [
                {"id": "t1", "preview": "fix the bug\nmore context", "updatedAt": 1789382089, "cwd": "/tmp/proj", "ephemeral": false},
                {"id": "t2", "name": "named thread", "preview": "ignored", "updatedAt": 5, "cwd": "/x", "ephemeral": false},
                {"id": "t3", "preview": "", "ephemeral": false},
                {"id": "t4", "preview": "our own turn", "ephemeral": true},
                {"id": "", "preview": "no id"},
            ],
            "nextCursor": "page-2",
        });
        let (sessions, cursor) = parse_thread_page(&result);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["t1", "t2", "t3"], "ephemeral and id-less threads drop out");
        assert_eq!(sessions[0].title, "fix the bug");
        assert_eq!(sessions[0].updated, 1789382089);
        assert_eq!(sessions[0].cwd, "/tmp/proj");
        assert_eq!(sessions[1].title, "named thread", "name beats preview");
        assert_eq!(sessions[2].title, "Untitled thread");
        assert_eq!(cursor, Some(json!("page-2")));
    }

    #[test]
    fn parse_thread_page_last_page_has_no_cursor() {
        let (sessions, cursor) = parse_thread_page(&json!({"data": [], "nextCursor": null}));
        assert!(sessions.is_empty());
        assert_eq!(cursor, None);
    }
}
