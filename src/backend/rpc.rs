//! `codex app-server` wire format: request builders and replies to
//! server-initiated requests. Pure `serde_json::Value` construction —
//! no I/O, so every shape is unit-testable.

use serde_json::{Value, json};

use super::{ApprovalDecision, ApprovalKind, ApprovalRoute};

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

/// Thread-level overrides shared by `thread/start` and `thread/resume`:
/// the model plus the sandbox/approval policy, and the merged custom
/// instructions carried as `developerInstructions` (additive — the
/// server's own base instructions still apply).
pub(crate) struct ThreadOpts<'a> {
    pub model: &'a str,
    pub sandbox: &'a str,
    pub approval: &'a str,
    /// Merged global + project instructions; `None` omits the field.
    pub instructions: Option<&'a str>,
}

/// `thread/start`: one persistent thread per chat — the chat binds the
/// returned id (`AgentEvent::ThreadBound`) so later sends `thread/resume`
/// it, in-session and across restarts. `cwd` is the thread's working
/// directory (project root or its worktree).
pub(crate) fn thread_start_req(id: i64, cwd: &std::path::Path, opts: &ThreadOpts<'_>) -> Value {
    let mut params = json!({
        "approvalPolicy": opts.approval,
        "sandbox": opts.sandbox,
        "ephemeral": false,
        "cwd": cwd.to_string_lossy(),
        "model": opts.model,
    });
    if let Some(instructions) = opts.instructions {
        params["developerInstructions"] = json!(instructions);
    }
    json!({"method": "thread/start", "id": id, "params": params})
}

/// `turn/start`: the user's prompt as a text input plus one `localImage`
/// input per image attachment (the `UserInput` variant that takes a local
/// path — the server reads the file itself). `effort` is the app-server's
/// `ReasoningEffort` override — `None` lets the thread keep the model's
/// `defaultReasoningEffort`.
pub(crate) fn turn_start_req(id: i64, thread_id: &str, prompt: &str, effort: Option<&str>, images: &[std::path::PathBuf]) -> Value {
    let mut input = vec![json!({"type": "text", "text": prompt, "text_elements": []})];
    for path in images {
        input.push(json!({"type": "localImage", "path": path.to_string_lossy()}));
    }
    let mut params = json!({
        "threadId": thread_id,
        "input": input,
    });
    if let Some(e) = effort {
        params["effort"] = json!(e);
    }
    json!({"method": "turn/start", "id": id, "params": params})
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

/// `account/read` — the signed-in account (or null) plus whether the
/// server requires OpenAI auth at all.
pub(crate) fn account_read_req(id: i64) -> Value {
    json!({"method": "account/read", "id": id, "params": {}})
}

/// `account/login/start` with the device-code flow — the response carries
/// `verificationUrl` + `userCode` to show the user; completion arrives as
/// the `account/login/completed` notification.
pub(crate) fn login_start_req(id: i64) -> Value {
    json!({"method": "account/login/start", "id": id, "params": {"type": "chatgptDeviceCode"}})
}

/// `account/logout` — clears the stored credentials.
pub(crate) fn logout_req(id: i64) -> Value {
    json!({"method": "account/logout", "id": id})
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

/// `thread/resume` — reopen a past thread. `opts` overrides the thread's
/// stored model/sandbox/approval and re-asserts the merged instructions so
/// a continued turn honors the chat's current configuration; `None` is a
/// read-only fetch.
pub(crate) fn thread_resume_req(id: i64, thread_id: &str, opts: Option<&ThreadOpts<'_>>) -> Value {
    let mut params = json!({"threadId": thread_id});
    if let Some(opts) = opts {
        params["model"] = json!(opts.model);
        params["sandbox"] = json!(opts.sandbox);
        params["approvalPolicy"] = json!(opts.approval);
        if let Some(instructions) = opts.instructions {
            params["developerInstructions"] = json!(instructions);
        }
    }
    json!({"method": "thread/resume", "id": id, "params": params})
}
/// `thread/compact/start` — ask the server to fold the thread's history
/// into a summary. Runs as its own turn: `item/*` notifications for the
/// `contextCompaction` item, `thread/tokenUsage/updated`, then
/// `turn/completed` + `thread/compacted`.
pub(crate) fn thread_compact_start_req(id: i64, thread_id: &str) -> Value {
    json!({"method": "thread/compact/start", "id": id, "params": {"threadId": thread_id}})
}

/// Parse one `thread/list` result into `(sessions, next_cursor)`. Ephemeral
/// threads are dropped — they hold no history worth reopening.
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
                        default_effort: m["defaultReasoningEffort"].as_str().unwrap_or("").into(),
                        efforts: m["supportedReasoningEfforts"]
                            .as_array()
                            .map(|opts| opts.iter().filter_map(|o| o["reasoningEffort"].as_str().map(Into::into)).collect())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (models, result["nextCursor"].as_str().map(|s| json!(s)))
}

/// The codex `ReviewDecision` wire value for a UI decision — shared by the
/// v2 `item/*/requestApproval` methods and the legacy `*Approval` ones.
fn review_decision(d: ApprovalDecision) -> &'static str {
    match d {
        ApprovalDecision::Approve => "approved",
        ApprovalDecision::Deny => "denied",
        ApprovalDecision::ApproveForSession => "approved_for_session",
    }
}

/// Whether a server-initiated method is an approval request the user can
/// answer — the rest (elicitation, user input) still get canned replies.
fn is_approval(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" | "applyPatchApproval" | "execCommandApproval"
    )
}

/// Card kind + detail for an approval request, or `None` when the method
/// isn't an approval. `command` arrives as a string or an argv array;
/// patch requests summarize their `fileChanges`/`files` keys.
pub(crate) fn approval_detail(method: &str, params: &Value) -> Option<(ApprovalKind, String)> {
    let kind = match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => ApprovalKind::Command,
        "item/fileChange/requestApproval" | "applyPatchApproval" => ApprovalKind::Patch,
        _ => return None,
    };
    let detail = match kind {
        ApprovalKind::Command => command_text(params),
        ApprovalKind::Patch => patch_text(params),
        _ => unreachable!(),
    };
    Some((kind, detail))
}

/// Command text from an approval's params: `command` may be a string or
/// an argv array; `reason` fills in when no command is carried.
fn command_text(params: &Value) -> String {
    if let Some(cmd) = params["command"].as_str() {
        return cmd.to_string();
    }
    if let Some(argv) = params["command"].as_array() {
        return argv.iter().filter_map(|a| a.as_str()).collect::<Vec<_>>().join(" ");
    }
    params["reason"].as_str().unwrap_or("").to_string()
}

/// Patch summary: the changed paths when `fileChanges` is an object keyed
/// by path (or a `files` array), else the request's `reason`.
fn patch_text(params: &Value) -> String {
    if let Some(files) = params["fileChanges"].as_object() {
        return files.keys().cloned().collect::<Vec<_>>().join("\n");
    }
    if let Some(files) = params["files"].as_array() {
        let names: Vec<&str> = files.iter().filter_map(|f| f.as_str()).collect();
        if !names.is_empty() {
            return names.join("\n");
        }
    }
    params["reason"].as_str().unwrap_or("").to_string()
}

/// JSON-RPC response for a server-initiated request. Approval methods get
/// the user's `decision`; elicitation is declined; everything else gets a
/// generic error so the server can't hang waiting on us.
pub(crate) fn request_reply(method: &str, msg: &Value, decision: ApprovalDecision) -> Value {
    let id = msg["id"].clone();
    let result = match method {
        m if is_approval(m) => json!({"decision": review_decision(decision)}),
        "mcpServer/elicitation/request" => json!({"action": "decline", "content": null, "_meta": null}),
        _ => {
            return json!({"id": id, "error": {"code": -32603, "message": format!("rixlcode cannot answer {method}")}});
        },
    };
    json!({"id": id, "result": result})
}

/// An approval request waiting on the UI: the original server message plus
/// the channel the card's responder feeds. `answer` blocks the pump until
/// the user decides — or the responder drops (cancel), which denies.
pub(crate) struct PendingApproval {
    pub msg: Value,
    pub rx: std::sync::mpsc::Receiver<ApprovalDecision>,
}

impl PendingApproval {
    /// Block for the UI's decision and write the JSON-RPC response.
    /// A dropped responder (stop, chat deleted, quit) answers Deny so the
    /// server never sees an approval it didn't get.
    pub fn answer(self, stdin: &mut dyn std::io::Write) -> Result<(), String> {
        let decision = self.rx.recv().unwrap_or(ApprovalDecision::Deny);
        let method = self.msg["method"].as_str().unwrap_or("");
        let reply = request_reply(method, &self.msg, decision);
        writeln!(stdin, "{reply}").map_err(|e| format!("codex stdin: {e}"))
    }
}

/// Route a server-initiated request: approvals become an
/// `ApprovalRequest` event + `PendingApproval` under `Ask`, an immediate
/// reply under `Auto`; everything else gets a canned reply. Returns
/// `(events, response, pending)` for the decoder's `Decoded`.
pub(crate) fn route_request(
    route: ApprovalRoute, method: &str, msg: &Value,
) -> (Vec<crate::backend::AgentEvent>, Option<Value>, Option<PendingApproval>) {
    let Some((kind, detail)) = approval_detail(method, &msg["params"]) else {
        return (vec![], Some(request_reply(method, msg, ApprovalDecision::Deny)), None);
    };
    match route {
        ApprovalRoute::Auto(decision) => (vec![], Some(request_reply(method, msg, decision)), None),
        ApprovalRoute::Ask => {
            let (respond, rx) = std::sync::mpsc::channel();
            let key = msg["params"]["itemId"].as_str().unwrap_or("").to_string()
                + msg["params"]["callId"].as_str().unwrap_or("")
                + &msg["id"].to_string();
            let ev = crate::backend::AgentEvent::ApprovalRequest {
                ix: crate::backend_parse::item_ix(&json!({"id": key})),
                kind,
                detail: detail.into(),
                respond,
            };
            (vec![ev], None, Some(PendingApproval { msg: msg.clone(), rx }))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_start_maps_sandbox_and_approval() {
        let cwd = std::path::Path::new("/tmp/thread-wt");
        let req = thread_start_req(
            2,
            cwd,
            &ThreadOpts {
                model: "gpt-5",
                sandbox: "workspace-write",
                approval: "on-failure",
                instructions: None,
            },
        );
        assert_eq!(req["method"], json!("thread/start"));
        assert_eq!(req["params"]["sandbox"], json!("workspace-write"));
        assert_eq!(req["params"]["approvalPolicy"], json!("on-failure"));
        assert_eq!(req["params"]["ephemeral"], json!(false), "chat threads persist so later sends can resume them");
        // The thread's working directory goes on the wire — a worktree
        // thread's server must see the worktree, not the process cwd.
        assert_eq!(req["params"]["cwd"], json!("/tmp/thread-wt"));
    }

    #[test]
    fn thread_start_always_sends_the_model() {
        // No synthetic "default" — the concrete id always goes on the wire.
        let cwd = std::path::Path::new("/tmp");
        let opts = ThreadOpts {
            model: "gpt-5",
            sandbox: "read-only",
            approval: "on-request",
            instructions: None,
        };
        assert_eq!(thread_start_req(2, cwd, &opts)["params"]["model"], json!("gpt-5"));
    }

    #[test]
    fn turn_start_wraps_prompt_as_text_input() {
        let req = turn_start_req(3, "tid", "hello", None, &[]);
        assert_eq!(req["params"]["threadId"], json!("tid"));
        assert_eq!(req["params"]["input"][0]["type"], json!("text"));
        assert_eq!(req["params"]["input"][0]["text"], json!("hello"));
    }

    #[test]
    fn turn_start_sends_effort_only_when_set() {
        // The app-server's `effort` override rides turn/start — set it and
        // the param lands; unset it and the param is absent entirely so the
        // thread keeps the model's defaultReasoningEffort.
        let req = turn_start_req(3, "tid", "hello", Some("high"), &[]);
        assert_eq!(req["params"]["effort"], json!("high"));
        let unset = turn_start_req(3, "tid", "hello", None, &[]);
        assert!(unset["params"].get("effort").is_none(), "unset effort must not reach the wire");
    }

    #[test]
    fn turn_start_appends_local_image_inputs() {
        let images = vec![std::path::PathBuf::from("/tmp/shot.png"), std::path::PathBuf::from("/tmp/diagram.webp")];
        let req = turn_start_req(3, "tid", "look", None, &images);
        let input = &req["params"]["input"];
        assert_eq!(input[0], json!({"type": "text", "text": "look", "text_elements": []}));
        assert_eq!(input[1], json!({"type": "localImage", "path": "/tmp/shot.png"}));
        assert_eq!(input[2], json!({"type": "localImage", "path": "/tmp/diagram.webp"}));
        assert_eq!(input.as_array().unwrap().len(), 3);
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
    fn parse_model_page_captures_reasoning_efforts() {
        let result = json!({
            "data": [
                {
                    "id": "gpt-6",
                    "displayName": "GPT-6",
                    "defaultReasoningEffort": "medium",
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "low", "description": "fast"},
                        {"reasoningEffort": "high", "description": "deep"},
                    ],
                },
                {"id": "gpt-5"},
            ],
        });
        let (models, _) = parse_model_page(&result);
        assert_eq!(models[0].default_effort.as_ref(), "medium");
        let efforts: Vec<&str> = models[0].efforts.iter().map(|e| e.as_ref()).collect();
        assert_eq!(efforts, ["low", "high"]);
        // Models without the fields parse with empty effort metadata.
        assert_eq!(models[1].default_effort.as_ref(), "");
        assert!(models[1].efforts.is_empty());
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
        let opts = ThreadOpts {
            model: "gpt-5",
            sandbox: "workspace-write",
            approval: "never",
            instructions: None,
        };
        let req = thread_resume_req(2, "tid-1", Some(&opts));
        assert_eq!(req["method"], json!("thread/resume"));
        assert_eq!(req["params"]["threadId"], json!("tid-1"));
        assert_eq!(req["params"]["model"], json!("gpt-5"));
        assert_eq!(req["params"]["sandbox"], json!("workspace-write"));
        assert_eq!(req["params"]["approvalPolicy"], json!("never"));
        // A bare resume (history fetch) carries only the thread id.
        let bare = thread_resume_req(3, "tid-1", None);
        assert_eq!(bare["params"], json!({"threadId": "tid-1"}));
    }

    #[test]
    fn thread_compact_start_sends_thread_id() {
        let req = thread_compact_start_req(3, "tid-1");
        assert_eq!(req["method"], json!("thread/compact/start"));
        assert_eq!(req["id"], json!(3));
        assert_eq!(req["params"], json!({"threadId": "tid-1"}));
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
