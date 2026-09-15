//! `claude` CLI transport: `claude -p --output-format stream-json --verbose`.
//!
//! One process per turn: the prompt goes over stdin (positional argv can't
//! safely hold arbitrary text), stdout streams NDJSON events decoded by
//! `claude_parse`. `--include-partial-messages` adds raw API stream events
//! so text renders token-by-token; `--verbose` is required for stream-json.
//! `--no-session-persistence` keeps one-shot turns out of the user's
//! `claude` session history — we never resume.

use std::io::Write;

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
            })
            .collect()
    }

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
struct ClaudeTurn {
    prompt: String,
    model: String,
    mode: String,
    /// Filesystem access for Agent turns — snapshotted at send time so a
    /// mid-turn settings change can't alter a running turn's permissions.
    access: super::AccessMode,
    /// The thread's working directory — the project root, or its git
    /// worktree when the thread runs in one.
    cwd: std::path::PathBuf,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    /// Set when the UI drops the stream — checked before spawn so a
    /// cancelled turn can't start a fresh child.
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
fn build_command(turn: &ClaudeTurn) -> std::process::Command {
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
///   headless `-p` auto-denies, so the turn stays read-only).
/// - Agent + auto-accept-edits → `--permission-mode acceptEdits` (file
///   edits auto-accepted, other actions still gated).
/// - Agent + auto → `--permission-mode acceptEdits` — claude can't confine
///   shell commands to the workspace, so workspace-write is the closest
///   "auto" it offers.
/// - Agent + full-access → `--dangerously-skip-permissions` (no prompts).
fn permission_args(turn: &ClaudeTurn) -> Vec<&'static str> {
    match (turn.mode == "Agent", turn.access) {
        (false, _) => vec!["--permission-mode", "plan"],
        (true, super::AccessMode::Supervised) => vec!["--permission-mode", "default"],
        (true, super::AccessMode::AutoAcceptEdits | super::AccessMode::Auto) => {
            vec!["--permission-mode", "acceptEdits"]
        },
        (true, super::AccessMode::FullAccess) => vec!["--dangerously-skip-permissions"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(mode: &str, access: super::super::AccessMode) -> ClaudeTurn {
        ClaudeTurn {
            prompt: "hi".into(),
            model: "sonnet".into(),
            mode: mode.into(),
            access,
            cwd: std::path::PathBuf::from("/tmp/thread-wt"),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn agent_maps_access_to_permission_flags() {
        use super::super::AccessMode;
        let cases = [
            (AccessMode::Supervised, vec!["--permission-mode", "default"]),
            (AccessMode::AutoAcceptEdits, vec!["--permission-mode", "acceptEdits"]),
            (AccessMode::Auto, vec!["--permission-mode", "acceptEdits"]),
            (AccessMode::FullAccess, vec!["--dangerously-skip-permissions"]),
        ];
        for (access, want) in cases {
            assert_eq!(permission_args(&turn("Agent", access)), want);
        }
    }

    #[test]
    fn command_spawns_in_the_thread_workdir() {
        use super::super::AccessMode;
        let cmd = build_command(&turn("Agent", AccessMode::Auto));
        assert_eq!(cmd.get_current_dir(), Some(std::path::Path::new("/tmp/thread-wt")));
    }

    #[test]
    fn plan_and_ask_stay_read_only() {
        for mode in ["Plan", "Ask"] {
            for access in super::super::AccessMode::ALL {
                assert_eq!(permission_args(&turn(mode, access)), vec!["--permission-mode", "plan"]);
            }
        }
    }
}
