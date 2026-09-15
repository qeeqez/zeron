//! ACP (Agent Client Protocol) transport: spawn an ACP agent subprocess,
//! run the `initialize` → `session/new` → `session/prompt` handshake over
//! stdio JSON-RPC, and stream `session/update` notifications as
//! `AgentEvent`s. Works with Zed-style agents such as
//! `npx -y @zed-industries/claude-code-acp`.

use std::io::{BufRead, Write};

use serde_json::Value;

use super::acp_decode::AcpDecoder;
use super::acp_rpc as wire;
use super::{AgentBackend, AgentEvent, ReplyStream, kill_slot};
use crate::model::ModelInfo;

/// Backend that shells out to an ACP agent over stdio JSON-RPC.
pub struct AcpBackend {
    /// Spawn command split into program + args (whitespace-separated).
    command: Vec<String>,
    /// The instance's Variables — injected into the spawned agent.
    env: Vec<(String, String)>,
    /// Model catalog learned from `session/new` responses — shared with
    /// every turn so `models()` reflects what the agent last advertised.
    models: std::sync::Arc<parking_lot::Mutex<Vec<ModelInfo>>>,
}

impl AcpBackend {
    /// Default agent command — the Zed claude-code ACP adapter.
    pub const DEFAULT_COMMAND: &'static str = "npx -y @zed-industries/claude-code-acp";

    /// `command` is a shell-style "program arg…" string; empty falls back
    /// to the default so a cleared setting can't silently wedge sends.
    pub fn new(command: String, env: Vec<(String, String)>) -> Self {
        let trimmed = command.trim();
        let command = if trimmed.is_empty() { Self::DEFAULT_COMMAND } else { trimmed };
        Self {
            command: command.split_whitespace().map(str::to_string).collect(),
            env,
            models: Default::default(),
        }
    }
}

impl Default for AcpBackend {
    fn default() -> Self {
        Self::new(String::new(), Vec::new())
    }
}

impl AgentBackend for AcpBackend {
    fn name(&self) -> &'static str {
        "acp"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.models.lock().clone()
    }

    fn send(&self, prompt: &str, model: &str, mode: &str, ctx: &super::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        // Each turn owns its child slot — concurrent chats can't clobber it.
        let turn = std::sync::Arc::new(AcpTurn {
            command: self.command.clone(),
            prompt: prompt.to_string(),
            model: model.to_string(),
            mode: mode.to_string(),
            access: ctx.access,
            cwd: ctx.cwd.clone(),
            images: ctx.images.clone(),
            // Snapshot the configured MCP servers — `session/new` advertises
            // them so the agent spawns/connects them for this session.
            mcp_servers: crate::persist::load_settings().mcp_servers,
            env: self.env.clone(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            models: self.models.clone(),
        });
        let thread_turn = turn.clone();
        std::thread::spawn(move || run_acp(&thread_turn, &tx));
        ReplyStream {
            events: rx,
            child: Some(turn.slot.clone()),
            cancelled: turn.cancelled.clone(),
        }
    }
}

/// Everything one ACP turn needs — bundled so the spawn helpers stay
/// under the argument-count lint.
pub(super) struct AcpTurn {
    command: Vec<String>,
    prompt: String,
    model: String,
    mode: String,
    /// Filesystem access snapshot — drives the permission/fs policy.
    access: super::AccessMode,
    /// Configured MCP servers handed to `session/new` — snapshotted at
    /// send time so a mid-turn settings change can't alter the session.
    mcp_servers: Vec<crate::mcp::McpServer>,
    /// Session working directory — `session/new`'s `cwd` and the
    /// workspace-write confinement root for `fs/write_text_file`.
    cwd: std::path::PathBuf,
    /// Image attachments — sent as `resource_link` blocks on `session/prompt`.
    images: Vec<std::path::PathBuf>,
    /// The instance's Variables — injected into the spawned agent.
    pub(super) env: Vec<(String, String)>,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Shared model catalog — `session/new` refreshes it for `models()`.
    pub(super) models: std::sync::Arc<parking_lot::Mutex<Vec<ModelInfo>>>,
}

#[cfg(test)]
impl AcpTurn {
    /// A turn over a fake command for pump/decoder tests — never spawned.
    pub(super) fn for_test(model: &str, mode: &str, access: super::AccessMode) -> Self {
        Self {
            mcp_servers: Vec::new(),
            command: vec!["acp-agent".into()],
            prompt: "hi".into(),
            model: model.into(),
            mode: mode.into(),
            access,
            cwd: std::path::PathBuf::from("/tmp"),
            images: Vec::new(),
            env: Vec::new(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            models: std::sync::Arc::new(parking_lot::Mutex::new(vec![])),
        }
    }

    /// A turn carrying configured MCP servers — `session/new` must
    /// advertise them.
    pub(super) fn with_mcp(mut self, servers: Vec<crate::mcp::McpServer>) -> Self {
        self.mcp_servers = servers;
        self
    }
}

fn run_acp(turn: &AcpTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    match spawn_acp(turn, tx) {
        AcpOutcome::Done | AcpOutcome::Cancelled | AcpOutcome::Dead => {},
        AcpOutcome::Failed(err) => {
            tx.send(AgentEvent::Error(err.into())).ok();
        },
    }
}

enum AcpOutcome {
    Done,
    Cancelled,
    Dead,
    Failed(String),
}

/// Handshake phase: which of our requests we're waiting on next. `Config`
/// carries the pending `session/set_mode` id (if any) so a model-select
/// round-trip doesn't lose it.
enum Phase {
    Init,
    Session,
    Config { sid: String, next_mode: Option<String> },
    Mode { sid: String },
    Prompt,
    Run,
}

/// How the pump ended — `spawn_acp` maps this onto an outcome.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(super) enum PumpEnd {
    /// `session/prompt` response arrived; the decoder emitted `Done`.
    Done,
    /// The event receiver or agent stdin is gone — stop quietly.
    Dead,
    /// Protocol or I/O failure worth an `Error` event.
    Failed(String),
    /// stdout hit EOF before the prompt response.
    Eof,
}

/// The agent spawn command for one turn — extracted so tests can assert
/// args and env without launching a real process.
pub(super) fn build_command(turn: &AcpTurn) -> std::process::Command {
    let mut cmd = std::process::Command::new(&turn.command[0]);
    cmd.args(&turn.command[1..])
        .current_dir(&turn.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    super::apply_env(&mut cmd, &turn.env);
    cmd
}

/// One ACP turn: spawn, handshake, stream `session/update` notifications
/// until the `session/prompt` response or EOF, reap, classify.
fn spawn_acp(turn: &AcpTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) -> AcpOutcome {
    let mut cmd = build_command(turn);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return AcpOutcome::Failed(format!("acp: couldn't start `{}` ({e}) — check the acp_command setting", turn.command.join(" ")));
        },
    };
    let stdout = child.stdout.take().expect("piped");
    let stdin = child.stdin.take().expect("piped");
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

    match pump(turn, std::io::BufReader::new(stdout), Box::new(stdin), tx) {
        // The session is over — reap now rather than on stream drop.
        PumpEnd::Done => {
            kill_slot(&turn.slot);
            AcpOutcome::Done
        },
        PumpEnd::Dead => {
            kill_slot(&turn.slot);
            AcpOutcome::Dead
        },
        PumpEnd::Failed(e) => {
            kill_slot(&turn.slot);
            AcpOutcome::Failed(e)
        },
        PumpEnd::Eof => {
            // EOF without a prompt response: killed by cancel() or crashed.
            let Some(mut child) = turn.slot.lock().take() else { return AcpOutcome::Cancelled };
            match child.wait() {
                Ok(s) if s.success() => AcpOutcome::Failed("acp agent exited without completing".into()),
                Ok(s) => {
                    let detail = stderr
                        .and_then(|h| h.join().ok())
                        .filter(|e| !e.is_empty())
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default();
                    AcpOutcome::Failed(format!("acp agent exited with {s}{detail}"))
                },
                Err(e) => AcpOutcome::Failed(format!("acp wait: {e}")),
            }
        },
    }
}

/// Read agent stdout line-by-line: responses advance the handshake, agent
/// requests get an immediate reply, `session/update` notifications decode
/// into `AgentEvent`s. Split from `spawn_acp` so tests can drive it with
/// canned NDJSON instead of a real subprocess.
pub(super) fn pump(turn: &AcpTurn, reader: impl BufRead, stdin: Box<dyn Write>, tx: &std::sync::mpsc::Sender<AgentEvent>) -> PumpEnd {
    let policy = wire::Policy::of(&turn.mode, turn.access, turn.cwd.clone());
    let mut hs = Handshake {
        phase: Phase::Init,
        seq: 1,
        stdin,
        decoder: AcpDecoder::new(),
    };
    if let Err(e) = hs.send(&wire::initialize_req(hs.seq, policy.write_fs)) {
        return PumpEnd::Failed(e);
    }
    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if msg.get("method").is_none() && msg.get("id").is_some() {
            // A response to one of our requests advances the phase machine.
            match hs.advance(turn, &msg) {
                Ok(Step::Next) => {},
                Ok(Step::Done(events)) => return drain_done(events, tx),
                Err(e) => return PumpEnd::Failed(e),
            }
            continue;
        }
        if msg.get("id").is_some() {
            if let Some(end) = agent_request(&msg, &mut hs, &policy, tx) {
                return end;
            }
            continue;
        }
        for e in hs.decoder.update(&msg["params"]["update"]) {
            if tx.send(e).is_err() {
                return PumpEnd::Dead;
            }
        }
    }
    PumpEnd::Eof
}

/// Emit the turn's final events; `Dead` when the receiver is gone.
fn drain_done(events: Vec<AgentEvent>, tx: &std::sync::mpsc::Sender<AgentEvent>) -> PumpEnd {
    if events.into_iter().any(|e| tx.send(e).is_err()) { PumpEnd::Dead } else { PumpEnd::Done }
}

/// Handle an agent→client request (`id` + `method`). Permission prompts in
/// "ask" modes surface as a card: emit the event, then block until the user
/// answers (a dropped responder — stop/delete — answers Deny). Other
/// requests (fs/*, unknown) get a canned reply so the turn can't hang.
/// `Some(end)` when the pump should stop.
fn agent_request(msg: &Value, hs: &mut Handshake, policy: &wire::Policy, tx: &std::sync::mpsc::Sender<AgentEvent>) -> Option<PumpEnd> {
    let method = msg["method"].as_str().unwrap_or("");
    if method == "session/request_permission" && matches!(policy.route, super::ApprovalRoute::Ask) {
        let (respond, rx) = std::sync::mpsc::channel();
        let tool_id = msg["params"]["toolCall"]["toolCallId"].as_str().unwrap_or("");
        let key = if tool_id.is_empty() { msg["id"].to_string() } else { tool_id.to_string() };
        let ev = AgentEvent::ApprovalRequest {
            ix: super::acp_decode::ix_of(&key),
            kind: super::ApprovalKind::Permission,
            detail: wire::permission_detail(&msg["params"]).into(),
            respond,
        };
        if tx.send(ev).is_err() {
            return Some(PumpEnd::Dead);
        }
        let decision = rx.recv().unwrap_or(super::ApprovalDecision::Deny);
        let reply = wire::permission_answer(msg["id"].clone(), &msg["params"], decision);
        if hs.send(&reply).is_err() {
            return Some(PumpEnd::Dead);
        }
        return None;
    }
    let reply = wire::request_reply(method, msg, policy);
    if hs.send(&reply).is_err() { Some(PumpEnd::Dead) } else { None }
}

/// Result of handling one response: keep pumping, or the turn is over
/// with these final events (`Done` plus any terminal error).
enum Step {
    Next,
    Done(Vec<AgentEvent>),
}

/// In-flight handshake state: the phase machine, request id sequence,
/// agent stdin, and the update decoder — bundled so the phase helpers
/// stay under the argument-count lint.
struct Handshake {
    phase: Phase,
    seq: i64,
    stdin: Box<dyn Write>,
    decoder: AcpDecoder,
}

impl Handshake {
    fn send(&mut self, v: &Value) -> Result<(), String> {
        wire::send(&mut *self.stdin, v)
    }

    /// Handle a response to one of our requests: send the next request in
    /// the `initialize` → `session/new` → [`set_config_option`] →
    /// [`set_mode`] → `session/prompt` sequence. Model and mode selection
    /// are best-effort — a failure there doesn't sink the turn.
    fn advance(&mut self, turn: &AcpTurn, msg: &Value) -> Result<Step, String> {
        match std::mem::replace(&mut self.phase, Phase::Run) {
            Phase::Init => {
                check_err(msg)?;
                self.seq += 1;
                self.send(&wire::session_new_req(self.seq, &turn.cwd.to_string_lossy(), crate::mcp::acp_mcp_servers(&turn.mcp_servers)))?;
                self.phase = Phase::Session;
            },
            Phase::Session => {
                check_err(msg)?;
                let result = &msg["result"];
                let sid = result["sessionId"].as_str().ok_or("acp: no sessionId")?.to_string();
                wire::cache_models(&turn.models, result);
                let next_mode = wire::mode_pick(&turn.mode, result);
                match wire::model_request(self.seq + 1, &sid, &turn.model, result) {
                    Some(req) => {
                        self.seq += 1;
                        self.send(&req)?;
                        self.phase = Phase::Config { sid, next_mode };
                    },
                    None => return self.chain_mode_or_prompt(turn, &sid, next_mode),
                }
            },
            Phase::Config { sid, next_mode } => return self.chain_mode_or_prompt(turn, &sid, next_mode),
            Phase::Mode { sid } => return self.send_prompt(turn, &sid),
            Phase::Prompt => {
                let mut out = self.decoder.close_open();
                if let Err(e) = check_err(msg) {
                    out.push(AgentEvent::Error(e.into()));
                } else if msg["result"]["stopReason"].as_str() == Some("refusal") {
                    out.push(AgentEvent::Error("acp: agent refused the prompt".into()));
                }
                out.push(AgentEvent::Done);
                return Ok(Step::Done(out));
            },
            Phase::Run => {},
        }
        Ok(Step::Next)
    }

    /// After `session/new` (or a config response): send `session/set_mode`
    /// when a mode was picked, otherwise go straight to the prompt.
    fn chain_mode_or_prompt(&mut self, turn: &AcpTurn, sid: &str, next_mode: Option<String>) -> Result<Step, String> {
        match next_mode {
            Some(mode_id) => {
                self.seq += 1;
                self.send(&wire::set_mode_req(self.seq, sid, &mode_id))?;
                self.phase = Phase::Mode { sid: sid.to_string() };
            },
            None => return self.send_prompt(turn, sid),
        }
        Ok(Step::Next)
    }

    fn send_prompt(&mut self, turn: &AcpTurn, sid: &str) -> Result<Step, String> {
        self.seq += 1;
        self.send(&wire::prompt_req(self.seq, sid, &turn.prompt, &turn.images))?;
        self.phase = Phase::Prompt;
        Ok(Step::Next)
    }
}

/// Extract the agent's error object, if the response carries one.
fn check_err(msg: &Value) -> Result<(), String> {
    match msg.get("error") {
        Some(err) => Err(format!("acp: {}", err["message"].as_str().unwrap_or("request failed"))),
        None => Ok(()),
    }
}
