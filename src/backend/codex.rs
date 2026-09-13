//! `codex exec --json` transport: spawn, stream JSONL, retry, kill.

use super::{AgentBackend, AgentEvent, ReplyStream, kill_slot};

/// Backend that shells out to `codex exec --json`.
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

    fn send(&self, prompt: &str, model: &str, mode: &str) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        // Each turn owns its child slot — concurrent chats can't clobber it.
        let turn = std::sync::Arc::new(CodexTurn {
            prompt: prompt.to_string(),
            model: model.to_string(),
            mode: mode.to_string(),
            access: super::access_mode(),
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
}

/// Everything one codex turn needs — bundled so the spawn helpers stay
/// under the argument-count lint.
struct CodexTurn {
    prompt: String,
    model: String,
    mode: String,
    /// Filesystem access for Agent turns — snapshotted at send time so a
    /// mid-turn settings change can't alter a running turn's sandbox.
    access: super::AccessMode,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    /// Set when the UI drops the stream — checked before each retry so a
    /// cancelled turn can't spawn a fresh child.
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
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

enum CodexOutcome {
    Done,
    Cancelled,
    Dead,
    Failed(String),
}

/// One `codex exec` attempt: spawn, read JSONL until EOF, reap, classify.
/// Returns the outcome plus whether any event was emitted.
fn spawn_codex(turn: &CodexTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) -> (CodexOutcome, bool) {
    let mut cmd = std::process::Command::new("codex");
    cmd.args(codex_args(turn))
        .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (CodexOutcome::Failed(format!("codex spawn: {e}")), false),
    };
    let stdout = child.stdout.take().expect("piped");
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
    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let (mut done, mut emitted) = (false, false);
    for line in reader.lines().map_while(Result::ok) {
        for e in crate::backend_parse::parse_codex_line(&line) {
            done |= matches!(e, AgentEvent::Done);
            emitted = true;
            if tx.send(e).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
    }
    if done {
        return (CodexOutcome::Done, emitted);
    }
    // EOF without turn.completed: killed by cancel() or crashed.
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

/// The `codex exec` argv for one turn. Plan/Ask stay read-only no matter
/// what the access setting says; Agent turns honor it — without this flag
/// `codex exec` defaults to read-only and can never write files.
fn codex_args(turn: &CodexTurn) -> Vec<&str> {
    let mut args = vec!["exec", "--json", "--skip-git-repo-check"];
    if turn.model != "default" {
        args.extend(["-m", turn.model.as_str()]);
    }
    args.extend(["-s", if turn.mode == "Agent" { turn.access.sandbox_arg() } else { "read-only" }]);
    args.push(turn.prompt.as_str());
    args
}

#[cfg(test)]
mod tests {
    use crate::backend::AccessMode;

    use super::*;

    fn turn(mode: &str, access: AccessMode) -> CodexTurn {
        CodexTurn {
            prompt: "hi".into(),
            model: "default".into(),
            mode: mode.into(),
            access,
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// The value passed to `-s`, and a check that it appears exactly once.
    fn sandbox_of<'a>(args: &[&'a str]) -> Option<&'a str> {
        assert_eq!(args.iter().filter(|a| **a == "-s").count(), 1);
        args.windows(2).find(|w| w[0] == "-s").map(|w| w[1])
    }

    #[test]
    fn agent_mode_gets_write_capable_sandbox() {
        // The bug this fixes: Agent sent no `-s` flag, so `codex exec`
        // defaulted to read-only and file_change events never happened.
        let cases = [
            (AccessMode::ReadOnly, "read-only"),
            (AccessMode::WorkspaceWrite, "workspace-write"),
            (AccessMode::FullAccess, "danger-full-access"),
        ];
        for (access, want) in cases {
            assert_eq!(sandbox_of(&codex_args(&turn("Agent", access))), Some(want));
        }
    }

    #[test]
    fn plan_and_ask_stay_read_only() {
        for mode in ["Plan", "Ask"] {
            for access in AccessMode::ALL {
                assert_eq!(sandbox_of(&codex_args(&turn(mode, access))), Some("read-only"));
            }
        }
    }

    #[test]
    fn model_flag_only_when_not_default() {
        let mut t = turn("Agent", AccessMode::WorkspaceWrite);
        assert!(!codex_args(&t).contains(&"-m"));
        t.model = "gpt-5".into();
        let args = codex_args(&t);
        let ix = args.iter().position(|a| *a == "-m").unwrap();
        assert_eq!(args[ix + 1], "gpt-5");
    }
}
