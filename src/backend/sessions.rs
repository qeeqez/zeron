//! `thread/list` + `thread/resume` over `codex app-server`: reopen past
//! threads so a chat can continue them.
//!
//! Separate from the turn transport in `codex.rs` — listing and reopening
//! need no turn and no event decoding; they handshake, request, and exit.
//! Runs on a background executor thread from `Workspace::refresh_sessions`
//! / `open_session` — never on the UI thread.

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::Value;

use super::rpc::{initialize_req, parse_thread_page, thread_list_req, thread_resume_req, thread_title};
use super::{ResumedSession, SessionInfo};
use crate::backend_parse::{item_ix, mcp_result_text, reasoning_text};
use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};

/// How long one session exchange may take before the child is killed —
/// a wedged app-server must not pin an executor thread forever.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Cap on `thread/list` pages — the sidebar shows recent threads, not the
/// whole history (25/page → at most 100 rows).
const MAX_PAGES: u32 = 4;

/// List past codex threads, newest first. Err on spawn failure, handshake
/// error, timeout, or EOF mid-list — the caller treats it as "no sessions".
pub fn fetch_codex_sessions(env: &[(String, String)]) -> Result<Vec<SessionInfo>, String> {
    exchange(env, |stdin, stdout| read_sessions(stdin, stdout))
}

/// Reopen one thread: `thread/resume` returns it with its recent turns,
/// mapped to chat messages for display.
pub fn resume_codex_session(thread_id: &str, env: &[(String, String)]) -> Result<ResumedSession, String> {
    let thread_id = thread_id.to_string();
    exchange(env, move |stdin, stdout| read_session(stdin, stdout, &thread_id))
}

/// Spawn `codex app-server`, run `drive` against its pipes on a helper
/// thread, and bound the exchange by `FETCH_TIMEOUT`. The child is killed
/// either way — these are one-shot queries, not a session.
pub(super) fn exchange<T: Send + 'static>(
    env: &[(String, String)], drive: impl FnOnce(&mut dyn Write, std::process::ChildStdout) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    exchange_cmd(&["codex".to_string(), "app-server".to_string()], env, "codex session", drive)
}

/// Spawn `argv` (an arbitrary JSON-RPC stdio server), run `drive` against
/// its pipes on a helper thread, and bound the exchange by
/// `FETCH_TIMEOUT`. `what` prefixes spawn/timeout errors ("codex
/// session", "mcp"). The child is killed either way — these are one-shot
/// queries, not a session.
pub(super) fn exchange_cmd<T: Send + 'static>(
    argv: &[String], env: &[(String, String)], what: &str,
    drive: impl FnOnce(&mut dyn Write, std::process::ChildStdout) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    super::apply_env(&mut cmd, env);
    let mut child = cmd.spawn().map_err(|e| format!("{what} spawn: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let mut stdin = child.stdin.take().expect("piped");

    // The read loop blocks on stdout, so it runs on its own thread — the
    // caller bounds the whole exchange with `recv_timeout` and kills the
    // child (closing stdout, ending the thread) on timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(drive(&mut stdin, stdout));
    });
    let result = rx.recv_timeout(FETCH_TIMEOUT).unwrap_or_else(|_| Err(format!("{what} request timed out")));
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// Handshake, then paginate `thread/list` until `nextCursor` is absent or
/// `MAX_PAGES` is reached. `stdin`/`stdout` are the app-server's pipes
/// (testable with in-memory cursors).
pub(super) fn read_sessions(stdin: &mut dyn Write, stdout: impl std::io::Read) -> Result<Vec<SessionInfo>, String> {
    let send = |stdin: &mut dyn Write, v: &Value| -> Result<(), String> { writeln!(stdin, "{v}").map_err(|e| format!("codex stdin: {e}")) };
    send(stdin, &initialize_req(1))?;

    let reader = std::io::BufReader::new(stdout);
    let mut sessions = Vec::new();
    // Request ids: 1 = initialize, 2.. = thread/list pages.
    let mut req_id = 1i64;
    let mut pages = 0u32;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        // Notifications and server-initiated requests carry `method`;
        // only responses to our requests advance the fetch.
        if msg.get("method").is_some() || msg.get("id").is_none() {
            continue;
        }
        if let Some(err) = msg.get("error") {
            let m = err["message"].as_str().unwrap_or("request failed");
            return Err(format!("codex: {m}"));
        }
        match msg["id"].as_i64() {
            Some(1) => {
                send(stdin, &serde_json::json!({"method": "initialized", "params": {}}))?;
                req_id += 1;
                send(stdin, &thread_list_req(req_id, None))?;
            },
            Some(id) if id == req_id => {
                let (page, next) = parse_thread_page(&msg["result"]);
                sessions.extend(page);
                pages += 1;
                match next.filter(|_| pages < MAX_PAGES) {
                    Some(cursor) => {
                        req_id += 1;
                        send(stdin, &thread_list_req(req_id, Some(&cursor)))?;
                    },
                    None => return Ok(sessions),
                }
            },
            _ => {},
        }
    }
    Err("codex closed stdout before thread/list completed".into())
}

/// Handshake, then `thread/resume` once — the result's `thread` carries
/// the recent turns.
pub(super) fn read_session(stdin: &mut dyn Write, stdout: impl std::io::Read, thread_id: &str) -> Result<ResumedSession, String> {
    let send = |stdin: &mut dyn Write, v: &Value| -> Result<(), String> { writeln!(stdin, "{v}").map_err(|e| format!("codex stdin: {e}")) };
    send(stdin, &initialize_req(1))?;

    let reader = std::io::BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if msg.get("method").is_some() || msg.get("id").is_none() {
            continue;
        }
        if let Some(err) = msg.get("error") {
            let m = err["message"].as_str().unwrap_or("request failed");
            return Err(format!("codex: {m}"));
        }
        match msg["id"].as_i64() {
            Some(1) => {
                send(stdin, &serde_json::json!({"method": "initialized", "params": {}}))?;
                send(stdin, &thread_resume_req(2, thread_id, None))?;
            },
            Some(2) => return parse_resumed(&msg["result"]),
            _ => {},
        }
    }
    Err("codex closed stdout before thread/resume completed".into())
}

/// Map a `thread/resume` result to a `ResumedSession`: identity plus the
/// transcript of its recent turns.
pub(super) fn parse_resumed(result: &Value) -> Result<ResumedSession, String> {
    let thread = &result["thread"];
    let id = thread["id"].as_str().ok_or("codex: no thread id")?.to_string();
    let mut messages = Vec::new();
    for turn in thread["turns"].as_array().into_iter().flatten() {
        for item in turn["items"].as_array().into_iter().flatten() {
            messages.extend(history_message(item));
        }
    }
    Ok(ResumedSession {
        id,
        title: thread_title(thread),
        cwd: thread["cwd"].as_str().unwrap_or("").to_string(),
        messages,
    })
}

/// One turn item → at most one chat message. Mirrors the live decoder's
/// item mapping in `appserver.rs` so resumed history renders identically.
pub(super) fn history_message(item: &Value) -> Option<ChatMessage> {
    let (role, kind) = match item["type"].as_str() {
        Some("userMessage") => {
            let text = item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["type"].as_str() == Some("text"))
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return None;
            }
            (Role::User, MessageKind::Text(text.into()))
        },
        Some("agentMessage") => {
            let text = item["text"].as_str().unwrap_or("");
            if text.is_empty() {
                return None;
            }
            (Role::Assistant, MessageKind::Text(text.into()))
        },
        _ => (Role::Assistant, MessageKind::Tool(tool_card(item)?)),
    };
    Some(ChatMessage {
        alternatives: vec![],
        role,
        kind,
        rating: None,
        bookmarked: false,
        pinned: false,
        at: std::time::SystemTime::now(),
        usage: None,
        attachments: vec![],
    })
}

/// A completed tool card for a history item — same name/detail mapping as
/// `TurnDecoder::item_started`, output from the item's final payload.
/// `None` for items with nothing to show (empty reasoning, unknown kinds).
pub(super) fn tool_card(item: &Value) -> Option<ToolCall> {
    let (name, detail) = match item["type"].as_str() {
        Some("commandExecution") => ("shell", item["command"].as_str().unwrap_or("")),
        Some("reasoning") => ("thinking", ""),
        Some("mcpToolCall") => (item["server"].as_str().unwrap_or("mcp"), item["tool"].as_str().unwrap_or("")),
        Some("dynamicToolCall") => ("tool", item["tool"].as_str().unwrap_or("")),
        Some("collabAgentToolCall") => ("agent", item["tool"].as_str().unwrap_or("")),
        Some("webSearch") => ("web_search", item["query"].as_str().unwrap_or("")),
        Some("fileChange") => ("file_change", ""),
        Some("plan") => ("plan", item["text"].as_str().unwrap_or("")),
        _ => return None,
    };
    let output = match item["type"].as_str() {
        Some("commandExecution") => item["aggregatedOutput"].as_str().unwrap_or("").to_string(),
        Some("reasoning") => reasoning_text(item),
        Some("mcpToolCall") | Some("dynamicToolCall") | Some("collabAgentToolCall") => mcp_result_text(item),
        Some("fileChange") => item["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["path"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if detail.is_empty() && output.is_empty() {
        return None;
    }
    Some(ToolCall {
        tool_ix: item_ix(item),
        name: name.into(),
        detail: detail.into(),
        output: output.into(),
        status: if item["status"].as_str() == Some("completed") { ToolStatus::Done } else { ToolStatus::Failed },
        expanded: false,
    })
}
