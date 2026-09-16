//! MCP (Model Context Protocol) transport: spawn an MCP server
//! subprocess, run `initialize` → `tools/list`/`prompts/list` →
//! `tools/call`/`prompts/get` over stdio JSON-RPC, and stream the result
//! as `AgentEvent`s.
//!
//! MCP servers expose *tools*, not chat — so the provider's "models" are
//! the server's tools and prompt templates (`tool:<name>`,
//! `prompt:<name>`), and a send is one call whose content blocks become
//! the reply. A server with neither gets a single pseudo-model named
//! after it that answers with its `serverInfo`.

use std::io::Write;

use super::mcp_session::{Session, SessionErr};
use super::{AgentBackend, AgentEvent, ReplyStream, kill_slot};
use crate::model::ModelInfo;

/// Backend that drives one MCP server over stdio JSON-RPC.
pub struct McpBackend {
    /// Spawn command split into argv (quote-aware, like the MCP server
    /// settings editor). Empty = unconfigured — sends fail fast.
    command: Vec<String>,
    /// The instance's Variables — injected into the spawned server.
    env: Vec<(String, String)>,
    /// Catalog learned from the last turn's listing — `models()` reflects
    /// what the server advertised so the picker has something before the
    /// next `fetch`.
    models: std::sync::Arc<parking_lot::Mutex<Vec<ModelInfo>>>,
}

impl McpBackend {
    pub fn new(command: String, env: Vec<(String, String)>) -> Self {
        Self {
            command: crate::mcp::split_command(&command),
            env,
            models: Default::default(),
        }
    }
}

impl AgentBackend for McpBackend {
    fn name(&self) -> &'static str {
        "mcp"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.models.lock().clone()
    }

    fn send(&self, prompt: &str, model: &str, _mode: &str, ctx: &super::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let turn = std::sync::Arc::new(McpTurn {
            command: self.command.clone(),
            // MCP has no system channel — merged instructions prefix the
            // prompt like ACP/HTTP do.
            prompt: crate::instructions::prefixed(prompt, ctx.instructions.as_deref()),
            model: model.to_string(),
            cwd: ctx.cwd.clone(),
            env: self.env.clone(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            models: self.models.clone(),
        });
        let thread_turn = turn.clone();
        std::thread::spawn(move || run_mcp(&thread_turn, &tx));
        ReplyStream {
            events: rx,
            child: Some(turn.slot.clone()),
            cancelled: turn.cancelled.clone(),
        }
    }
}

/// Everything one MCP turn needs — bundled so the spawn helpers stay
/// under the argument-count lint.
pub(super) struct McpTurn {
    command: Vec<String>,
    prompt: String,
    /// The picker's pseudo-model id — `tool:<name>`, `prompt:<name>`, or
    /// the server name.
    model: String,
    /// The server's working directory.
    cwd: std::path::PathBuf,
    /// The instance's Variables — injected into the spawned server.
    pub(super) env: Vec<(String, String)>,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Shared catalog — each turn's listing refreshes it for `models()`.
    pub(super) models: std::sync::Arc<parking_lot::Mutex<Vec<ModelInfo>>>,
}

#[cfg(test)]
impl McpTurn {
    /// A turn over a fake command for session tests — never spawned.
    pub(super) fn for_test(model: &str) -> Self {
        Self {
            command: vec!["mcp-server".into()],
            prompt: "hi".into(),
            model: model.into(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: Vec::new(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            models: std::sync::Arc::new(parking_lot::Mutex::new(vec![])),
        }
    }
}

fn run_mcp(turn: &McpTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if turn.command.is_empty() {
        tx.send(AgentEvent::Error("mcp: no server command configured — set it in the provider's settings".into()))
            .ok();
        return;
    }
    match spawn_mcp(turn, tx) {
        McpOutcome::Done | McpOutcome::Cancelled | McpOutcome::Dead => {},
        McpOutcome::Failed(err) => {
            tx.send(AgentEvent::Error(err.into())).ok();
        },
    }
}

enum McpOutcome {
    Done,
    Cancelled,
    Dead,
    Failed(String),
}

/// The server spawn command for one turn — extracted so tests can assert
/// args and env without launching a real process.
pub(super) fn build_command(turn: &McpTurn) -> std::process::Command {
    let mut cmd = std::process::Command::new(&turn.command[0]);
    cmd.args(&turn.command[1..])
        .current_dir(&turn.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    super::apply_env(&mut cmd, &turn.env);
    cmd
}

/// One MCP turn: spawn, handshake, list, call, reap, classify.
fn spawn_mcp(turn: &McpTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) -> McpOutcome {
    let mut cmd = build_command(turn);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return McpOutcome::Failed(format!("mcp: couldn't start `{}` ({e}) — check the provider's command", turn.command.join(" ")));
        },
    };
    let stdout = child.stdout.take().expect("piped");
    let stdin = child.stdin.take().expect("piped");
    // Drain stderr on a thread from spawn — a chatty server blocks on a
    // full pipe before stdout EOF, and we want the text on failure.
    let stderr = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut buf);
            buf.trim().to_string()
        })
    });
    *turn.slot.lock() = Some(child);

    match drive(turn, stdout, stdin, tx) {
        Ok(()) => {
            kill_slot(&turn.slot);
            McpOutcome::Done
        },
        Err(SessionErr::Dead) => {
            kill_slot(&turn.slot);
            McpOutcome::Dead
        },
        Err(SessionErr::Eof) => {
            // EOF mid-request: killed by cancel() or crashed.
            let Some(mut child) = turn.slot.lock().take() else { return McpOutcome::Cancelled };
            match child.wait() {
                Ok(s) if s.success() => McpOutcome::Failed("mcp server exited without completing".into()),
                Ok(s) => {
                    let detail = stderr
                        .and_then(|h| h.join().ok())
                        .filter(|e| !e.is_empty())
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default();
                    McpOutcome::Failed(format!("mcp server exited with {s}{detail}"))
                },
                Err(e) => McpOutcome::Failed(format!("mcp wait: {e}")),
            }
        },
        Err(SessionErr::Failed(e)) => {
            kill_slot(&turn.slot);
            McpOutcome::Failed(e)
        },
    }
}

/// The turn's wire sequence over any pipes — testable with in-memory
/// cursors: handshake + list, resolve the model, run the call, `Done`.
/// Events go to `tx`; `Err(Dead)` when the receiver is gone.
pub(super) fn drive<'a>(
    turn: &McpTurn, stdout: impl std::io::Read + 'a, stdin: impl Write + 'a, tx: &std::sync::mpsc::Sender<AgentEvent>,
) -> Result<(), SessionErr> {
    let mut session = Session::new(stdout, stdin);
    let mut emit = |e: AgentEvent| tx.send(e).is_ok();
    let listings = session.listings()?;
    *turn.models.lock() = listings.models();
    let target = listings.resolve(&turn.model, &turn.prompt);
    session.run(&target, &listings, &mut emit)?;
    tx.send(AgentEvent::Done).map_err(|_| SessionErr::Dead)
}

/// The instance's catalog fetch: spawn the server, handshake, list, and
/// return the pseudo-models. `Err` on spawn failure, handshake error,
/// timeout, or EOF — callers fall back to the cached catalog.
pub fn fetch_mcp_tools(p: &crate::providers::ProviderInstance) -> Result<Vec<ModelInfo>, String> {
    let argv = crate::mcp::split_command(&p.command);
    if argv.is_empty() {
        return Err("mcp: no server command configured".into());
    }
    super::sessions::exchange_cmd(&argv, &p.env, "mcp", |stdin, stdout| {
        let mut session = Session::new(stdout, stdin);
        session.listings().map(|l| l.models()).map_err(|e| match e {
            SessionErr::Dead => "mcp: connection lost".to_string(),
            SessionErr::Eof => "mcp: server closed stdout mid-listing".to_string(),
            SessionErr::Failed(e) => e,
        })
    })
}
