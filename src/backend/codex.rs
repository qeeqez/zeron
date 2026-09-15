//! `codex app-server` transport: spawn, NDJSON-RPC handshake, stream deltas.
//!
//! The real Codex desktop app talks to `codex app-server` over stdio
//! JSON-RPC — unlike `codex exec --json`, it streams `agentMessage` and
//! command-output deltas, so replies render token-by-token.

use std::io::Write;

use serde_json::{Value, json};

use super::appserver::TurnDecoder;
use super::rpc::{initialize_req, thread_resume_req, thread_start_req, turn_start_req};
use super::{AgentBackend, AgentEvent, ReplyStream, kill_slot};

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
        crate::model::codex_fallback_models()
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
            resume: ctx.thread_id.clone(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
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
    prompt: String,
    model: String,
    mode: String,
    /// Filesystem access for Agent turns — snapshotted at send time so a
    /// mid-turn settings change can't alter a running turn's sandbox.
    access: super::AccessMode,
    /// The thread's working directory — the project root, or its git
    /// worktree when the thread runs in one.
    cwd: std::path::PathBuf,
    /// Resume this codex thread instead of starting an ephemeral one — set
    /// on chats bound to a past session.
    resume: Option<String>,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    /// Set when the UI drops the stream — checked before each retry so a
    /// cancelled turn can't spawn a fresh child.
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
            resume: None,
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
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
        if attempt > 0 { std::thread::sleep(std::time::Duration::from_millis(400 * attempt)); }
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

enum CodexOutcome {
    Done,
    Cancelled,
    Dead,
    Failed(String),
}

/// Handshake phase: which request id we're waiting on next.
pub(super) enum Phase {
    Init,
    Thread,
    Turn,
    Run,
}

/// One `codex app-server` attempt: spawn, handshake, stream notifications
/// until `turn/completed` or EOF, reap, classify.
/// The `codex app-server` command for one turn — extracted so tests can
/// assert the spawn cwd without launching a real process.
pub(super) fn build_command(turn: &CodexTurn) -> std::process::Command {
    let mut cmd = std::process::Command::new("codex");
    cmd.arg("app-server")
        .current_dir(&turn.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

fn spawn_codex(turn: &CodexTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) -> (CodexOutcome, bool) {
    let mut cmd = build_command(turn);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (CodexOutcome::Failed(format!("codex spawn: {e}")), false),
    };
    let stdout = child.stdout.take().expect("piped");
    let mut stdin = child.stdin.take().expect("piped");
    // Drain stderr on a thread from spawn — a chatty child blocks on a
    // full pipe before stdout EOF, and we want the text on failure.
    let stderr = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut buf);
            buf.trim().to_string()
        })
    });
    *turn.slot.lock() = Some(child);

    let send_req =
        |stdin: &mut dyn Write, v: &Value| -> Result<(), String> { writeln!(stdin, "{v}").map_err(|e| format!("codex stdin: {e}")) };
    if let Err(e) = send_req(&mut stdin, &initialize_req(1)) {
        kill_slot(&turn.slot);
        return (CodexOutcome::Failed(e), false);
    }

    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let mut decoder = TurnDecoder::new(approval_route_of(turn));
    let mut phase = Phase::Init;
    let (mut done, mut emitted) = (false, false);
    for line in reader.lines().map_while(Result::ok) {
        // Responses to our handshake requests advance the phase machine.
        if let Ok(msg) = serde_json::from_str::<Value>(&line)
            && msg.get("method").is_none()
            && msg.get("id").is_some()
        {
            match advance_phase(&mut phase, turn, &msg, &mut stdin) {
                Ok(true) => {},
                Ok(false) => continue,
                Err(e) => {
                    kill_slot(&turn.slot);
                    return (CodexOutcome::Failed(e), emitted);
                },
            }
            continue;
        }
        let decoded = decoder.line(&line);
        done |= decoded.turn_over;
        for e in decoded.events {
            emitted = true;
            if tx.send(e).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
        if let Some(pending) = decoded.pending {
            // The approval card is on screen — block until the user
            // answers (or the responder drops on cancel → Deny), then
            // write the decision back to the server.
            if pending.answer(&mut stdin).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
        if let Some(resp) = decoded.response {
            // Other server requests (elicitation, user input): answer
            // immediately so the turn can't hang waiting on us.
            if send_req(&mut stdin, &resp).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
        if done {
            break;
        }
    }
    if done {
        return (CodexOutcome::Done, emitted);
    }
    // EOF without turn/completed: killed by cancel() or crashed.
    let Some(mut child) = turn.slot.lock().take() else { return (CodexOutcome::Cancelled, emitted) };
    let outcome = match child.wait() {
        Ok(s) if s.success() => CodexOutcome::Failed("codex exited without completing".into()),
        Ok(s) => {
            let detail = stderr
                .and_then(|h| h.join().ok())
                .filter(|e| !e.is_empty())
                .map(|e| format!(": {e}"))
                .unwrap_or_default();
            CodexOutcome::Failed(format!("codex exited with {s}{detail}"))
        },
        Err(e) => CodexOutcome::Failed(format!("codex wait: {e}")),
    };
    (outcome, emitted)
}

/// Handle a response to one of our handshake requests: send the next
/// request in the sequence. Returns Ok(true) when the line was consumed.
pub(super) fn advance_phase(phase: &mut Phase, turn: &CodexTurn, msg: &Value, stdin: &mut dyn Write) -> Result<bool, String> {
    let id = msg["id"].as_i64().unwrap_or(-1);
    if let Some(err) = msg.get("error") {
        let m = err["message"].as_str().unwrap_or("request failed");
        return Err(format!("codex: {m}"));
    }
    match (std::mem::replace(phase, Phase::Run), id) {
        (Phase::Init, 1) => {
            // `initialized` notification, then open the turn's thread —
            // resume a bound session's thread, else start an ephemeral one.
            let open = match &turn.resume {
                Some(tid) => thread_resume_req(2, tid, Some(&turn.model), Some(sandbox_of(turn)), Some(approval_of(turn))),
                None => thread_start_req(2, &turn.model, sandbox_of(turn), approval_of(turn), &turn.cwd),
            };
            writeln!(stdin, "{}", json!({"method": "initialized", "params": {}}))
                .and_then(|()| writeln!(stdin, "{open}"))
                .map_err(|e| format!("codex stdin: {e}"))?;
            *phase = Phase::Thread;
            Ok(true)
        },
        (Phase::Thread, 2) => {
            let tid = msg["result"]["thread"]["id"].as_str().ok_or("codex: no thread id")?.to_string();
            writeln!(stdin, "{}", turn_start_req(3, &tid, &turn.prompt)).map_err(|e| format!("codex stdin: {e}"))?;
            *phase = Phase::Turn;
            Ok(true)
        },
        (Phase::Turn, 3) => {
            if msg["result"]["turn"]["id"].is_null() {
                return Err("codex: no turn id".into());
            }
            *phase = Phase::Run;
            Ok(true)
        },
        // Not a handshake response — restore the phase and let the
        // decoder see the line.
        (old, _) => {
            *phase = old;
            Ok(false)
        },
    }
}

/// Sandbox for `thread/start` — Agent honors the access setting, Plan/Ask
/// stay read-only, mirroring the old `codex exec -s` mapping.
pub(super) fn sandbox_of(turn: &CodexTurn) -> &'static str {
    if turn.mode == "Agent" { turn.access.sandbox_arg() } else { "read-only" }
}

/// `approvalPolicy` for `thread/start` — Agent honors the access setting's
/// ask/auto split; Plan/Ask never prompt (their sandbox is read-only).
pub(super) fn approval_of(turn: &CodexTurn) -> &'static str {
    if turn.mode == "Agent" { turn.access.approval_arg() } else { "never" }
}

/// How this turn answers approval requests: Agent mode honors the access
/// setting's ask/auto split; Plan/Ask deny outright — their read-only
/// sandbox must never see an approval it could grant.
pub(super) fn approval_route_of(turn: &CodexTurn) -> super::ApprovalRoute {
    if turn.mode == "Agent" { turn.access.approval_route() } else { super::ApprovalRoute::Auto(super::ApprovalDecision::Deny) }
}
