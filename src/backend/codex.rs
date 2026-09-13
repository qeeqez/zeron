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
    let mut args = vec!["exec", "--json", "--skip-git-repo-check"];
    if turn.model != "default" {
        args.extend(["-m", turn.model.as_str()]);
    }
    // Plan/Ask are read-only turns — the agent must not write files.
    if matches!(turn.mode.as_str(), "Plan" | "Ask") {
        args.extend(["-s", "read-only"]);
    }
    args.push(turn.prompt.as_str());
    cmd.args(&args)
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
