use std::pin::Pin;

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
    /// The run finished normally.
    Done,
    /// The run failed; `message` is human-readable.
    Error(SharedString),
}

pub struct ReplyStream {
    /// Events as they arrive.
    pub events: Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>>,
}

/// Pluggable agent backend. Implementations live behind `dyn` so the UI
/// can swap transports without touching chat state.
pub trait AgentBackend: Send + Sync {
    /// Human-readable name for the status bar.
    fn name(&self) -> &'static str;
    /// Start a reply turn. The returned stream yields events until
    /// `Done`/`Error` or cancellation.
    fn send(&self, prompt: &str, model: &str, mode: &str) -> ReplyStream;
    /// Cancel the in-flight turn, if any.
    fn cancel(&self);
}

/// Simulated backend: emits a canned event stream (tool call, diff, text).
pub struct SimBackend;

impl AgentBackend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str) -> ReplyStream {
        let events = futures::stream::iter([
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
        ]);
        ReplyStream { events: Box::pin(events) }
    }

    fn cancel(&self) {}
}

/// Backend that shells out to `codex exec --json`.
pub struct CodexCliBackend {
    child: std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
}

impl CodexCliBackend {
    pub fn new() -> Self {
        Self { child: std::sync::Arc::new(parking_lot::Mutex::new(None)) }
    }
}

impl AgentBackend for CodexCliBackend {
    fn name(&self) -> &'static str {
        "codex-cli"
    }

    fn send(&self, prompt: &str, model: &str, _mode: &str) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let slot = self.child.clone();
        let (prompt, model) = (prompt.to_string(), model.to_string());
        std::thread::spawn(move || run_codex(&prompt, &model, &slot, &tx));
        let stream = futures::stream::poll_fn(move |_| match rx.try_recv() {
            Ok(e) => std::task::Poll::Ready(Some(e)),
            Err(std::sync::mpsc::TryRecvError::Empty) => std::task::Poll::Pending,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => std::task::Poll::Ready(None),
        });
        ReplyStream { events: Box::pin(stream) }
    }

    fn cancel(&self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Spawn `codex exec`, stream its JSONL events, retry on empty failed exits.
fn run_codex(
    prompt: &str, model: &str, slot: &std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    tx: &std::sync::mpsc::Sender<AgentEvent>,
) {
    let mut emitted = false;
    for attempt in 0..3u64 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400 * attempt));
        }
        let (outcome, got_events) = spawn_codex(prompt, model, slot, tx);
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
fn spawn_codex(
    prompt: &str, model: &str, slot: &std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>,
    tx: &std::sync::mpsc::Sender<AgentEvent>,
) -> (CodexOutcome, bool) {
    let mut cmd = std::process::Command::new("codex");
    cmd.args(["exec", "--json", "--skip-git-repo-check", "-m", model, prompt])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (CodexOutcome::Failed(format!("codex spawn: {e}")), false),
    };
    let stdout = child.stdout.take().expect("piped");
    *slot.lock() = Some(child);
    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let (mut done, mut emitted) = (false, false);
    for line in reader.lines().map_while(Result::ok) {
        let Some(e) = parse_codex_line(&line) else { continue };
        done |= matches!(e, AgentEvent::Done);
        emitted = true;
        if tx.send(e).is_err() {
            if let Some(mut c) = slot.lock().take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            return (CodexOutcome::Dead, emitted);
        }
    }
    if done {
        return (CodexOutcome::Done, emitted);
    }
    // EOF without turn.completed: killed by cancel() or crashed.
    let Some(mut child) = slot.lock().take() else { return (CodexOutcome::Cancelled, emitted) };
    let outcome = match child.wait() {
        Ok(s) if s.success() => CodexOutcome::Failed("codex exited without completing".into()),
        Ok(s) => CodexOutcome::Failed(format!("codex exited with {s}")),
        Err(e) => CodexOutcome::Failed(format!("codex wait: {e}")),
    };
    (outcome, emitted)
}

/// Map one `codex exec --json` JSONL line to an `AgentEvent`.
fn parse_codex_line(line: &str) -> Option<AgentEvent> {
    let ev = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let item = &ev["item"];
    match ev["type"].as_str()? {
        "item.started" if item["type"].as_str() == Some("command_execution") => Some(AgentEvent::ToolCallStart {
            ix: 0,
            name: "shell".into(),
            detail: item["command"].as_str().unwrap_or("").into(),
        }),
        "item.completed" => match item["type"].as_str()? {
            "command_execution" => Some(AgentEvent::ToolCallEnd { ix: 0, ok: item["exit_code"].as_i64() == Some(0) }),
            "agent_message" => Some(AgentEvent::TextDelta(item["text"].as_str().unwrap_or("").into())),
            _ => None,
        },
        "turn.completed" => Some(AgentEvent::Done),
        _ => None,
    }
}
