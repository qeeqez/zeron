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
