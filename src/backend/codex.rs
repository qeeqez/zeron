//! `codex app-server` transport: spawn, NDJSON-RPC handshake, stream deltas.
//!
//! The real Codex desktop app talks to `codex app-server` over stdio
//! JSON-RPC — unlike `codex exec --json`, it streams `agentMessage` and
//! command-output deltas, so replies render token-by-token.

use super::codex_turn::{CodexOutcome, spawn_codex};
use super::steer::CodexSlot;
use super::{AgentBackend, AgentEvent, ReplyStream};

/// Backend that shells out to `codex app-server` (the desktop transport).
pub struct CodexCliBackend;

impl CodexCliBackend {
    pub fn new() -> Self {
        Self
    }
}

impl AgentBackend for CodexCliBackend {
    fn name(&self) -> &'static str {
        "codex-cli"
    }

    /// Static catalog — the live list comes from `model/list` via
    /// `fetch_codex_models` and lands on the workspace's catalog.
    fn models(&self) -> Vec<crate::model::ModelInfo> {
        crate::model_catalog::codex_fallback_models()
    }

    fn send(&self, prompt: &str, model: &str, mode: &str, ctx: &super::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        // Each turn owns its child slot — concurrent chats can't clobber it.
        let turn = std::sync::Arc::new(CodexTurn {
            prompt: prompt.to_string(),
            model: model.to_string(),
            mode: mode.to_string(),
            access: ctx.access,
            cwd: ctx.cwd.clone(),
            images: ctx.images.clone(),
            resume: ctx.thread_id.clone(),
            effort: ctx.effort.clone(),
            slot: std::sync::Arc::new(CodexSlot::new()),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let thread_turn = turn.clone();
        std::thread::spawn(move || run_codex(&thread_turn, &tx));
        ReplyStream {
            events: rx,
            child: Some(turn.slot.clone()),
            cancelled: turn.cancelled.clone(),
        }
    }

    /// `turn/steer` rides the live app-server connection — codex turns can
    /// take user input mid-turn.
    fn supports_steer(&self) -> bool {
        true
    }

    fn supports_sessions(&self) -> bool {
        true
    }

    /// `thread/list` — past codex threads for the sidebar's Resume section.
    fn list_sessions(&self) -> Option<Vec<super::SessionInfo>> {
        super::sessions::fetch_codex_sessions().ok()
    }

    /// `thread/resume` — reopen the thread and return its transcript.
    fn resume_session(&self, thread_id: &str) -> Option<super::ResumedSession> {
        super::sessions::resume_codex_session(thread_id).ok()
    }
}

/// Everything one codex turn needs — bundled so the spawn helpers stay
/// under the argument-count lint.
pub(super) struct CodexTurn {
    pub(super) prompt: String,
    pub(super) model: String,
    pub(super) mode: String,
    /// Filesystem access for Agent turns — snapshotted at send time so a
    /// mid-turn settings change can't alter a running turn's sandbox.
    pub(super) access: super::AccessMode,
    /// The thread's working directory — the project root, or its git
    /// worktree when the thread runs in one.
    pub(super) cwd: std::path::PathBuf,
    /// Image attachments — sent as `localImage` inputs on `turn/start`.
    pub(super) images: Vec<std::path::PathBuf>,
    /// Resume this codex thread instead of starting an ephemeral one — set
    /// on chats bound to a past session.
    pub(super) resume: Option<String>,
    /// Reasoning effort for `turn/start` — `None` lets the server apply
    /// the model's `defaultReasoningEffort`.
    pub(super) effort: Option<String>,
    /// The turn's live handle: child slot, shared stdin, and the
    /// thread/turn ids `turn/steer` addresses.
    pub(super) slot: std::sync::Arc<CodexSlot>,
    /// Set when the UI drops the stream — checked before each retry so a
    /// cancelled turn can't spawn a fresh child.
    pub(super) cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl CodexTurn {
    /// A turn for mapping/command tests — never spawned.
    pub(super) fn for_test(mode: &str, access: super::AccessMode) -> Self {
        Self {
            prompt: "hi".into(),
            model: "gpt-5".into(),
            mode: mode.into(),
            access,
            cwd: std::path::PathBuf::from("/tmp/thread-wt"),
            effort: None,
            images: Vec::new(),
            resume: None,
            slot: std::sync::Arc::new(CodexSlot::new()),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// A turn bound to an existing codex thread — the handshake resumes it.
    pub(super) fn resuming(mut self, thread_id: &str) -> Self {
        self.resume = Some(thread_id.to_string());
        self
    }
}

fn run_codex(turn: &CodexTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    let mut emitted = false;
    for attempt in 0..3u64 {
        // A dropped stream set the flag — don't spawn a fresh child.
        if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400 * attempt));
        }
        let (outcome, got_events) = spawn_codex(turn, tx);
        emitted |= got_events;
        match outcome {
            CodexOutcome::Done | CodexOutcome::Cancelled | CodexOutcome::Dead => return,
            CodexOutcome::Failed(err) => {
                if emitted || attempt == 2 {
                    tx.send(AgentEvent::Error(err.into())).ok();
                    return;
                }
            },
        }
    }
}

/// How long one `mcpServerStatus/list` fetch may take before the child is
/// killed — same bound as the model/session one-shot queries.
const STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Cap on `mcpServerStatus/list` pages — the settings list is small.
const STATUS_MAX_PAGES: u32 = 4;

/// Live MCP server status from `codex app-server` — the settings section's
/// per-server status dots. Err on spawn failure, handshake error, timeout,
/// or EOF mid-list; the caller treats it as "status unknown".
pub fn fetch_mcp_status() -> Result<Vec<crate::mcp::McpStatus>, String> {
    let mut cmd = std::process::Command::new("codex");
    cmd.arg("app-server")
        .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("codex spawn: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let mut stdin = child.stdin.take().expect("piped");

    // The read loop blocks on stdout, so it runs on its own thread — the
    // caller bounds the whole exchange with `recv_timeout` and kills the
    // child (closing stdout, ending the thread) on timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read_mcp_status(&mut stdin, stdout));
    });
    let result = rx
        .recv_timeout(STATUS_TIMEOUT)
        .unwrap_or_else(|_| Err("codex mcpServerStatus/list timed out".into()));
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// `mcpServerStatus/list` — one page of server statuses. `cursor` is the
/// previous page's `nextCursor`; `None` requests the first page.
fn mcp_status_req(id: i64, cursor: Option<&serde_json::Value>) -> serde_json::Value {
    let mut params = serde_json::json!({"limit": 100});
    if let Some(c) = cursor {
        params["cursor"] = c.clone();
    }
    serde_json::json!({"method": "mcpServerStatus/list", "id": id, "params": params})
}

/// Handshake, then paginate `mcpServerStatus/list` until `nextCursor` is
/// absent or `STATUS_MAX_PAGES` is reached. `stdin`/`stdout` are the
/// app-server's pipes (testable with in-memory cursors).
pub(crate) fn read_mcp_status(stdin: &mut dyn std::io::Write, stdout: impl std::io::Read) -> Result<Vec<crate::mcp::McpStatus>, String> {
    use std::io::BufRead;
    let send = |stdin: &mut dyn std::io::Write, v: &serde_json::Value| -> Result<(), String> {
        writeln!(stdin, "{v}").map_err(|e| format!("codex stdin: {e}"))
    };
    send(stdin, &super::rpc::initialize_req(1))?;

    let reader = std::io::BufReader::new(stdout);
    let mut statuses = Vec::new();
    // Request ids: 1 = initialize, 2.. = mcpServerStatus/list pages.
    let mut req_id = 1i64;
    let mut pages = 0u32;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
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
                send(stdin, &mcp_status_req(req_id, None))?;
            },
            Some(id) if id == req_id => {
                let (page, next) = crate::mcp::parse_status_page(&msg["result"]);
                statuses.extend(page);
                pages += 1;
                match next.filter(|_| pages < STATUS_MAX_PAGES) {
                    Some(cursor) => {
                        req_id += 1;
                        send(stdin, &mcp_status_req(req_id, Some(&cursor)))?;
                    },
                    None => return Ok(statuses),
                }
            },
            _ => {},
        }
    }
    Err("codex closed stdout before mcpServerStatus/list completed".into())
}
