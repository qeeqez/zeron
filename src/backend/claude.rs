//! `claude` CLI transport: `claude -p --output-format stream-json --verbose`.
//!
//! One process per turn: the prompt goes over stdin (positional argv can't
//! safely hold arbitrary text), stdout streams NDJSON events decoded by
//! `claude_parse`. `--include-partial-messages` adds raw API stream events
//! so text renders token-by-token; `--verbose` is required for stream-json.
//! `--no-session-persistence` keeps one-shot turns out of the user's
//! `claude` session history — we never resume.

use std::io::Write;
use std::sync::mpsc::Sender;

use super::claude_parse::ClaudeDecoder;
use super::{AgentBackend, AgentEvent, ReplyStream, kill_slot};

/// Backend that shells out to the `claude` CLI (Claude Code print mode).
pub struct ClaudeCliBackend;

/// Models the picker offers for this provider — claude-cli can't enumerate
/// its models, so this is the static alias list from `claude --help`.
/// An empty model id omits `--model` so claude uses its configured default.
const CLAUDE_MODELS: [(&str, &str); 4] = [("sonnet", "Sonnet"), ("opus", "Opus"), ("haiku", "Haiku"), ("fable", "Fable")];

impl ClaudeCliBackend {
    pub fn new() -> Self {
        Self
    }
}

impl AgentBackend for ClaudeCliBackend {
    fn name(&self) -> &'static str {
        "claude-cli"
    }

    fn models(&self) -> Vec<crate::model::ModelInfo> {
        CLAUDE_MODELS
            .iter()
            .map(|(id, label)| crate::model::ModelInfo {
                id: (*id).into(),
                label: (*label).into(),
                description: "".into(),
                ..Default::default()
            })
            .collect()
    }

    /// `claude -p` takes the prompt on stdin — no image input on the wire,
    /// so `ctx.images` stay path references in the prompt's `[Attached
    /// files:]` list (the model reads them with its file tools).
    fn send(&self, prompt: &str, model: &str, mode: &str, ctx: &super::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        // Each turn owns its child slot — concurrent chats can't clobber it.
        let turn = std::sync::Arc::new(ClaudeTurn {
            prompt: prompt.to_string(),
            model: model.to_string(),
            mode: mode.to_string(),
            access: ctx.access,
            cwd: ctx.cwd.clone(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let thread_turn = turn.clone();
        std::thread::spawn(move || run_claude(&thread_turn, &tx));
        ReplyStream {
            events: rx,
            child: Some(turn.slot.clone()),
            cancelled: turn.cancelled.clone(),
        }
    }
}

/// Everything one claude turn needs — bundled so the spawn helpers stay
/// under the argument-count lint.
pub(super) struct ClaudeTurn {
    pub(super) prompt: String,
    pub(super) model: String,
    pub(super) mode: String,
    /// Filesystem access for Agent turns — snapshotted at send time so a
    /// mid-turn settings change can't alter a running turn's permissions.
    pub(super) access: super::AccessMode,
    /// The thread's working directory — the project root, or its git
    /// worktree when the thread runs in one.
    pub(super) cwd: std::path::PathBuf,
    pub(super) slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    /// Set when the UI drops the stream — checked before spawn so a
    /// cancelled turn can't start a fresh child.
    pub(super) cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// One attempt, no retry loop: a failed `claude -p` turn may already have
/// run tools, so blindly re-running it could repeat side effects.
fn run_claude(turn: &ClaudeTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let ClaudeOutcome::Failed(err) = spawn_claude(turn, tx) {
        tx.send(AgentEvent::Error(err.into())).ok();
    }
}

enum ClaudeOutcome {
    Done,
    /// Stream dropped or child killed by cancel — stay quiet.
    Cancelled,
    Failed(String),
}

/// One `claude -p` turn: spawn, write the prompt to stdin, stream NDJSON
/// lines through the decoder until `result` or EOF, reap, classify.
/// The `claude -p` command for one turn — extracted so tests can assert
/// args and the spawn cwd without launching a real process.
pub(super) fn build_command(turn: &ClaudeTurn) -> std::process::Command {
    let mut cmd = std::process::Command::new("claude");
    cmd.arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--no-session-persistence")
        .args(permission_args(turn))
        .current_dir(&turn.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if !turn.model.is_empty() {
        cmd.arg("--model").arg(&turn.model);
    }
    cmd
}

fn spawn_claude(turn: &ClaudeTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) -> ClaudeOutcome {
    let mut cmd = build_command(turn);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ClaudeOutcome::Failed(
                "claude CLI not found — install Claude Code (`brew install claude` or `npm i -g @anthropic-ai/claude-code`) or pick another backend".into(),
            );
        },
        Err(e) => return ClaudeOutcome::Failed(format!("claude spawn: {e}")),
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

    if let Err(e) = stdin.write_all(turn.prompt.as_bytes()).and_then(|()| stdin.flush()) {
        kill_slot(&turn.slot);
        return ClaudeOutcome::Failed(format!("claude stdin: {e}"));
    }
    drop(stdin); // close stdin so claude reads the prompt and starts

    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let mut decoder = ClaudeDecoder::new();
    let mut done = false;
    for line in reader.lines().map_while(Result::ok) {
        let decoded = decoder.line(&line);
        done |= decoded.turn_over;
        for e in decoded.events {
            if tx.send(e).is_err() {
                kill_slot(&turn.slot);
                return ClaudeOutcome::Cancelled;
            }
        }
        if done {
            break;
        }
    }
    if done {
        // `result` is the last line — the process is exiting; reap it now.
        if let Some(mut c) = turn.slot.lock().take() {
            let _ = c.wait();
        }
        return ClaudeOutcome::Done;
    }
    // EOF without `result`: killed by cancel() or crashed.
    let Some(mut child) = turn.slot.lock().take() else { return ClaudeOutcome::Cancelled };
    match child.wait() {
        Ok(s) if s.success() => ClaudeOutcome::Failed("claude exited without a result".into()),
        Ok(s) => {
            let detail = stderr
                .and_then(|h| h.join().ok())
                .filter(|e| !e.is_empty())
                .map(|e| format!(": {e}"))
                .unwrap_or_default();
            ClaudeOutcome::Failed(format!("claude exited with {s}{detail}"))
        },
        Err(e) => ClaudeOutcome::Failed(format!("claude wait: {e}")),
    }
}

/// Map mode + access to claude's permission flags. Claude has no sandbox
/// levels like codex's `-s`; the closest mapping:
/// - Plan/Ask → `--permission-mode plan` (read-only planning mode).
/// - Agent + supervised → `--permission-mode default` (every action asks;
///   headless `-p` emits no answerable prompt — the CLI auto-denies, so
///   the turn stays read-only. Unlike codex/ACP there's no wire to route
///   an `ApprovalRequest` over, so the card can't help here).
/// - Agent + auto-accept-edits → `--permission-mode acceptEdits` (file
///   edits auto-accepted, other actions still gated).
/// - Agent + auto → `--permission-mode acceptEdits` — claude can't confine
///   shell commands to the workspace, so workspace-write is the closest
///   "auto" it offers.
/// - Agent + full-access → `--dangerously-skip-permissions` (no prompts).
pub(super) fn permission_args(turn: &ClaudeTurn) -> Vec<&'static str> {
    match (turn.mode == "Agent", turn.access) {
        (false, _) => vec!["--permission-mode", "plan"],
        (true, super::AccessMode::Supervised) => vec!["--permission-mode", "default"],
        (true, super::AccessMode::AutoAcceptEdits | super::AccessMode::Auto) => {
            vec!["--permission-mode", "acceptEdits"]
        },
        (true, super::AccessMode::FullAccess) => vec!["--dangerously-skip-permissions"],
    }
}

// ── Auth: `claude auth status/login/logout` ──

/// `claude auth status --json` → the instance's sign-in state. Blocking —
/// call off the UI thread.
pub(crate) fn auth_status() -> crate::auth::AuthState {
    match std::process::Command::new("claude").args(["auth", "status", "--json"]).output() {
        Ok(o) => parse_auth_status(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => crate::auth::AuthState::Unknown,
    }
}

/// Map `claude auth status --json` output to a state. `loggedIn` decides;
/// the detail prefers the account email/org, then the auth method.
pub(super) fn parse_auth_status(out: &str) -> crate::auth::AuthState {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(out) else {
        return crate::auth::AuthState::Unknown;
    };
    match v["loggedIn"].as_bool() {
        Some(true) => {
            let detail = ["email", "orgName", "subscriptionType", "authMethod"]
                .iter()
                .filter_map(|k| v[k].as_str())
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .to_string();
            crate::auth::AuthState::SignedIn(detail)
        },
        Some(false) => crate::auth::AuthState::SignedOut,
        None => crate::auth::AuthState::Unknown,
    }
}

/// `claude auth logout` — clears the CLI's stored credentials.
pub(crate) fn logout() -> Result<(), String> {
    let out = std::process::Command::new("claude")
        .args(["auth", "logout"])
        .output()
        .map_err(|e| format!("claude spawn: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start `claude auth login`: the CLI prints the OAuth URL, opens the
/// browser, then waits for the pasted code on stdin — the session's
/// `stdin` slot is how `submit_auth_code` answers it. The worker reports
/// `AuthEvent`s and re-probes status when the child exits.
pub(crate) fn login(tx: Sender<crate::auth::AuthEvent>) -> Result<crate::auth::LoginHandle, String> {
    let mut child = std::process::Command::new("claude")
        .args(["auth", "login"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("claude spawn: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let stdin = std::sync::Arc::new(parking_lot::Mutex::new(child.stdin.take()));
    let slot = std::sync::Arc::new(parking_lot::Mutex::new(Some(child)));
    let worker_slot = slot.clone();
    std::thread::spawn(move || {
        pump_login(stdout, &tx);
        let _ = tx.send(crate::auth::AuthEvent::Done(auth_status()));
        if let Some(mut child) = worker_slot.lock().take() {
            let _ = child.wait();
        }
    });
    Ok(crate::auth::LoginHandle { child: slot, stdin: Some(stdin) })
}

/// Read the login child's stdout until EOF, emitting `NeedsCode` once the
/// paste-back prompt appears (the URL rides along when it printed first).
/// Byte-wise because the "Paste code here" prompt has no trailing newline.
/// Testable with in-memory readers.
pub(super) fn pump_login(stdout: impl std::io::Read, tx: &Sender<crate::auth::AuthEvent>) {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let mut reader = std::io::BufReader::new(stdout);
    let mut asked = false;
    while reader.read(&mut byte).is_ok_and(|n| n == 1) {
        buf.push(byte[0]);
        // Scan on prompt-looking tails and periodically — the paste
        // prompt ends with "> " and no newline.
        if asked || (!buf.ends_with(b"> ") && buf.len() % 64 != 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&buf);
        if text.contains("Paste code") || text.contains("paste the code") {
            asked = true;
            let url = text.split_whitespace().find(|w| w.starts_with("https://")).unwrap_or("");
            let prompt = if url.is_empty() {
                "Paste the sign-in code below.".to_string()
            } else {
                format!("Open {url} to sign in, then paste the code below.")
            };
            let _ = tx.send(crate::auth::AuthEvent::NeedsCode(prompt));
        }
    }
}
