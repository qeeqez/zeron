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
    /// Token usage for the completed turn.
    Usage { input: u64, output: u64 },
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
        .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")))
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
        for e in parse_codex_line(&line) {
            done |= matches!(e, AgentEvent::Done);
            emitted = true;
            if tx.send(e).is_err() {
                kill_slot(slot);
                return (CodexOutcome::Dead, emitted);
            }
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

/// Kill and reap the child in the slot, if any.
fn kill_slot(slot: &parking_lot::Mutex<Option<std::process::Child>>) {
    if let Some(mut c) = slot.lock().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Map one `codex exec --json` JSONL line to zero or more `AgentEvent`s.
fn parse_codex_line(line: &str) -> Vec<AgentEvent> {
    let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else { return vec![] };
    let item = &ev["item"];
    let Some(kind) = ev["type"].as_str() else { return vec![] };
    match kind {
        "item.started" if item["type"].as_str() == Some("command_execution") => vec![AgentEvent::ToolCallStart {
            ix: 0,
            name: "shell".into(),
            detail: item["command"].as_str().unwrap_or("").into(),
        }],
        "item.completed" => match item["type"].as_str() {
            Some("command_execution") => {
                let mut out = Vec::with_capacity(2);
                let output = item["aggregated_output"].as_str().unwrap_or("");
                if !output.is_empty() {
                    out.push(AgentEvent::ToolCallDelta { ix: 0, output: output.into() });
                }
                out.push(AgentEvent::ToolCallEnd { ix: 0, ok: item["exit_code"].as_i64() == Some(0) });
                out
            },
            Some("agent_message") => vec![AgentEvent::TextDelta(item["text"].as_str().unwrap_or("").into())],
            Some("file_change") => file_change_events(item),
            Some("error") => vec![AgentEvent::Error(item["message"].as_str().unwrap_or("codex error").into())],
            _ => vec![],
        },
        "error" => vec![AgentEvent::Error(ev["message"].as_str().unwrap_or("codex error").into())],
        "turn.failed" => vec![AgentEvent::Error(ev["error"]["message"].as_str().unwrap_or("turn failed").into())],
        "turn.completed" => {
            let usage = &ev["usage"];
            let input = usage["input_tokens"].as_u64().unwrap_or(0);
            let output = usage["output_tokens"].as_u64().unwrap_or(0);
            vec![AgentEvent::Usage { input, output }, AgentEvent::Done]
        },
        _ => vec![],
    }
}

/// Turn a completed `file_change` item into `Diff` cards by asking git for
/// the working-tree diff of each touched path.
fn file_change_events(item: &serde_json::Value) -> Vec<AgentEvent> {
    if item["status"].as_str() != Some("completed") {
        return vec![];
    }
    let Some(changes) = item["changes"].as_array() else { return vec![] };
    changes.iter().filter_map(|c| c["path"].as_str()).filter_map(diff_for_path).collect()
}

/// `git diff` for `path` (or `--no-index` for untracked files), capped at
/// 200 lines so a huge generated file can't flood the chat.
fn diff_for_path(path: &str) -> Option<AgentEvent> {
    let tracked = std::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let output = if tracked {
        std::process::Command::new("git").args(["diff", "--", path]).output().ok()?
    } else {
        std::process::Command::new("git")
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .output()
            .ok()?
    };
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        return None;
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut hunks = String::new();
    let mut kept = 0usize;
    for line in text.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
        if kept < 200 {
            hunks.push_str(line);
            hunks.push('\n');
            kept += 1;
        }
    }
    Some(AgentEvent::Diff { path: path.into(), added, removed, hunks: hunks.into() })
}
