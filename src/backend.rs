use gpui_kit::SharedString;

/// Events streamed from an agent backend into a chat.
#[derive(Clone, Debug)]
pub enum AgentEvent {
    /// Incremental text for the in-flight assistant message.
    TextDelta(SharedString),
    /// A tool call started; `ix` is the message index it will occupy.
    ToolCallStart { ix: usize, name: SharedString, detail: SharedString },
    /// Streaming args/output for the tool call at `ix`.
    ToolCallDelta { ix: usize, output: SharedString },
    /// Tool call finished; `ok` flips status to Done/Failed.
    ToolCallEnd { ix: usize, ok: bool },
    /// A diff card to append.
    Diff { path: SharedString, added: usize, removed: usize, hunks: SharedString },
    /// Token usage for the completed turn.
    Usage { input: u64, output: u64 },
    /// The run finished normally.
    Done,
    /// The run failed; `message` is human-readable.
    Error(SharedString),
}

/// One reply turn's event channel plus the handle that kills its process.
/// Dropping the stream (task cancel, chat delete, quit) kills the child.
pub struct ReplyStream {
    /// Events as they arrive; `Err` on recv means the producer is gone.
    pub events: std::sync::mpsc::Receiver<AgentEvent>,
    /// Per-turn child slot; `None` for backends without a process.
    child: Option<std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>>,
}

impl Drop for ReplyStream {
    fn drop(&mut self) {
        if let Some(slot) = &self.child {
            kill_slot(slot);
        }
    }
}

/// Pluggable agent backend. Implementations live behind `dyn` so the UI
/// can swap transports without touching chat state.
pub trait AgentBackend: Send + Sync {
    /// Human-readable name for the status bar.
    fn name(&self) -> &'static str;
    /// Start a reply turn. The returned stream yields events until
    /// `Done`/`Error` or cancellation (drop the stream to cancel).
    fn send(&self, prompt: &str, model: &str, mode: &str) -> ReplyStream;
}
pub struct SimBackend;

impl AgentBackend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        for e in [
            AgentEvent::ToolCallStart { ix: 0, name: "cargo build".into(), detail: "--locked".into() },
            AgentEvent::ToolCallDelta { ix: 0, output: "   Compiling rixlcode v0.1.0\n".into() },
            AgentEvent::ToolCallEnd { ix: 0, ok: true },
            AgentEvent::Diff {
                path: "src/main.rs".into(),
                added: 24,
                removed: 6,
                hunks: "@@ -10,6 +10,24 @@\n fn main() {\n-    println!(\"old\");\n+    gpui_kit::application().run(|cx| {\n+        gpui_kit::init(cx);\n+    });\n }".into(),
            },
            AgentEvent::TextDelta("Done. The build is **green** — `0 warnings`, all checks passed.\n\n- `cargo build --locked` finished in 3.6s\n- clippy: clean\n- nextest: 0 tests".into()),
            AgentEvent::Done,
        ] {
            let _ = tx.send(e);
        }
        ReplyStream { events, child: None }
    }
}
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
        });
        let thread_turn = turn.clone();
        std::thread::spawn(move || run_codex(&thread_turn, &tx));
        ReplyStream { events: rx, child: Some(turn.slot.clone()) }
    }
}
/// Everything one codex turn needs — bundled so the spawn helpers stay
/// under the argument-count lint.
struct CodexTurn {
    prompt: String,
    model: String,
    mode: String,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
}

fn run_codex(turn: &CodexTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    let mut emitted = false;
    for attempt in 0..3u64 {
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
        .stderr(std::process::Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (CodexOutcome::Failed(format!("codex spawn: {e}")), false),
    };
    let stdout = child.stdout.take().expect("piped");
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
        Ok(s) => CodexOutcome::Failed(format!("codex exited with {s}")),
        Err(e) => CodexOutcome::Failed(format!("codex wait: {e}")),
    };
    (outcome, emitted)
}

/// Kill and reap the child in the slot, if any.
fn kill_slot(slot: &parking_lot::Mutex<Option<std::process::Child>>) {
    if let Some(mut c) = slot.lock().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}
