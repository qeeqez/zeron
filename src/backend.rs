use std::pin::Pin;

use gpui_kit::SharedString;

use crate::model::{ChatMessage, Role};

/// Events streamed from an agent backend into a chat.
#[allow(dead_code)] // exercised once a real transport lands
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

#[allow(dead_code)] // exercised once a real transport lands
pub struct ReplyStream {
    /// The user message that triggered this turn.
    pub prompt: ChatMessage,
    /// Events as they arrive.
    pub events: Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>>,
}

/// Pluggable agent backend. Implementations live behind `dyn` so the UI
#[allow(dead_code)] // exercised once a real transport lands
pub trait AgentBackend: Send + Sync {
    /// Human-readable name for the status bar.
    fn name(&self) -> &'static str;
    /// Available model ids for the picker.
    fn models(&self) -> &'static [&'static str];
    /// Start a reply turn. The returned stream yields events until
    /// `Done`/`Error` or cancellation.
    fn send(&self, prompt: &str, model: &str, mode: &str) -> ReplyStream;
    /// Cancel the in-flight turn, if any.
    fn cancel(&self);
}

#[allow(dead_code)] // exercised once a real transport lands
pub struct SimBackend;

impl AgentBackend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn models(&self) -> &'static [&'static str] {
        &["gpt-5-codex", "gpt-5", "gpt-5-mini"]
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str) -> ReplyStream {
        let prompt = ChatMessage {
            role: Role::User,
            kind: crate::model::MessageKind::Text(prompt.into()),
            rating: None,
            at: std::time::SystemTime::now(),
        };
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
        ReplyStream { prompt, events: Box::pin(events) }
    }

    fn cancel(&self) {}
}

/// Backend that shells out to `codex exec --json`.
pub struct CodexCliBackend {
    child: parking_lot::Mutex<Option<std::process::Child>>,
}

impl CodexCliBackend {
    pub fn new() -> Self {
        Self { child: parking_lot::Mutex::new(None) }
    }
}

impl AgentBackend for CodexCliBackend {
    fn name(&self) -> &'static str {
        "codex-cli"
    }

    fn models(&self) -> &'static [&'static str] {
        &["gpt-5-codex", "gpt-5", "gpt-5-mini"]
    }

    fn send(&self, prompt: &str, model: &str, _mode: &str) -> ReplyStream {
        let prompt_msg = ChatMessage {
            role: Role::User,
            kind: crate::model::MessageKind::Text(prompt.into()),
            rating: None,
            at: std::time::SystemTime::now(),
        };
        let mut cmd = std::process::Command::new("codex");
        cmd.args(["exec", "--json", "--skip-git-repo-check", "-m", model, prompt])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            return ReplyStream {
                prompt: prompt_msg,
                events: Box::pin(futures::stream::iter([AgentEvent::Error("codex not found".into())])),
            };
        };
        let stdout = child.stdout.take().expect("piped");
        *self.child.lock() = Some(child);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                parse_codex_line(&line).map(|e| tx.send(e).ok());
            }
        });
        let stream = futures::stream::poll_fn(move |_| match rx.try_recv() {
            Ok(e) => std::task::Poll::Ready(Some(e)),
            Err(std::sync::mpsc::TryRecvError::Empty) => std::task::Poll::Pending,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => std::task::Poll::Ready(None),
        });
        ReplyStream { prompt: prompt_msg, events: Box::pin(stream) }
    }

    fn cancel(&self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
        }
    }
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
