//! The built-in simulator backend — a canned turn for the `sim` provider,
//! split from `backend.rs` to stay under the SLOC cap.

use super::{AgentBackend, AgentEvent, ReplyStream, TurnContext};

/// The `sim` provider's backend: emits a fixed tool-call + diff + reply
/// stream so the UI is exercisable without a real agent.
pub struct SimBackend;

impl AgentBackend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    /// One static model so the simulator is selectable end-to-end — a
    /// provider with no catalog can't be sent to at all.
    fn models(&self) -> Vec<crate::model::ModelInfo> {
        vec![crate::model::ModelInfo {
            id: "sim".into(),
            label: "Sim".into(),
            description: "built-in simulator".into(),
            ..Default::default()
        }]
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        for e in [
            AgentEvent::ToolCallStart { ix: 0, name: "cargo build".into(), detail: "--locked".into() },
            AgentEvent::ToolCallDelta { ix: 0, output: "   Compiling rixlcode v0.1.0\n".into() },
            AgentEvent::ToolCallEnd { ix: 0, ok: true },
            AgentEvent::Diff {
                path: "src/main.rs".into(),
                added: 24,
                removed: 6,
                hunks: "@@ -10,6 +10,24 @@\n fn main() {\n-    println!(\"old\");\n+    gpui_kit::application().run(|cx| {\n        gpui_kit::init(cx);\n    });\n }".into(),
            },
            AgentEvent::TextDelta("Done. The build is **green** — `0 warnings`, all checks passed.\n\n- `cargo build --locked` finished in 3.6s\n- clippy: clean\n- nextest: 0 tests".into()),
            AgentEvent::Done,
        ] {
            let _ = tx.send(e);
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}
