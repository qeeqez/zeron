use gpui_kit::SharedString;

mod codex;
mod http;

pub use codex::CodexCliBackend;
pub use http::HttpBackend;

/// How much filesystem access an Agent-mode turn gets. Plan/Ask turns are
/// always read-only regardless of this setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    /// `-s read-only` — the agent can inspect but not modify files.
    ReadOnly,
    /// `-s workspace-write` — writes confined to the working directory.
    #[default]
    WorkspaceWrite,
    /// `-s danger-full-access` — unsandboxed, like Codex's full access.
    FullAccess,
}

impl AccessMode {
    /// All modes, in settings-picker order (least to most permissive).
    pub const ALL: [AccessMode; 3] = [AccessMode::ReadOnly, AccessMode::WorkspaceWrite, AccessMode::FullAccess];

    /// Stable id stored in settings.json.
    pub fn name(self) -> &'static str {
        match self {
            AccessMode::ReadOnly => "read-only",
            AccessMode::WorkspaceWrite => "workspace-write",
            AccessMode::FullAccess => "full-access",
        }
    }

    /// Label shown in the settings picker.
    pub fn label(self) -> &'static str {
        match self {
            AccessMode::ReadOnly => "Read only",
            AccessMode::WorkspaceWrite => "Workspace write",
            AccessMode::FullAccess => "Full access",
        }
    }

    /// `codex exec -s` value for this mode.
    pub fn sandbox_arg(self) -> &'static str {
        match self {
            AccessMode::ReadOnly => "read-only",
            AccessMode::WorkspaceWrite => "workspace-write",
            AccessMode::FullAccess => "danger-full-access",
        }
    }

    /// Parse a settings.json value; anything unknown falls back to the
    /// default so a stale or hand-edited file can't wedge the picker.
    pub fn from_name(name: &str) -> Self {
        Self::ALL.iter().copied().find(|m| m.name() == name).unwrap_or_default()
    }
}

/// Process-wide access level read by `CodexCliBackend::send`. The `send`
/// signature stays stable (its call sites live in files owned by other
/// lanes), so the workspace publishes the setting here instead — it also
/// survives `toggle_backend` rebuilding the backend mid-session.
static ACCESS_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(AccessMode::WorkspaceWrite as u8);

/// Publish the workspace's access mode for subsequent backend turns.
pub fn set_access_mode(mode: AccessMode) {
    ACCESS_MODE.store(mode as u8, std::sync::atomic::Ordering::Relaxed);
}

/// The access mode applied to the next backend turn.
pub fn access_mode() -> AccessMode {
    match ACCESS_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => AccessMode::ReadOnly,
        2 => AccessMode::FullAccess,
        _ => AccessMode::WorkspaceWrite,
    }
}

/// Events streamed from an agent backend into a chat.
#[derive(Clone, Debug)]
pub enum AgentEvent {
    /// Start a fresh assistant text bubble — the next TextDelta appends
    /// to it instead of the previous one. Emitted on `item.started` for
    /// `agent_message` so consecutive messages don't merge.
    TextStart,
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
    /// Per-turn child slot; `None` for backends without a process. Shared
    /// with the chat so stop/delete can kill a hung child directly —
    /// dropping the stream alone only cancels once the pump wakes.
    pub child: Option<std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>>,
    /// Set on drop so the backend's retry loop can't spawn a fresh child
    /// after cancellation.
    pub(crate) cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for ReplyStream {
    fn drop(&mut self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
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
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Kill and reap the child in the slot, if any.
pub(crate) fn kill_slot(slot: &parking_lot::Mutex<Option<std::process::Child>>) {
    if let Some(mut c) = slot.lock().take() {
        let _ = c.kill();
        // Reap off-thread — a child in uninterruptible sleep would block
        // the UI on wait().
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

/// Build the selected backend. `http` falls back to codex-cli when no
/// endpoint is configured — an empty URL would fail every send anyway.
pub fn make_backend(s: &crate::persist::Settings) -> std::sync::Arc<dyn AgentBackend> {
    match s.backend_name() {
        "sim" => std::sync::Arc::new(SimBackend),
        "http" if !s.http_url.is_empty() => std::sync::Arc::new(HttpBackend::new(s.http_url.clone(), s.http_key_env.clone())),
        _ => std::sync::Arc::new(CodexCliBackend::new()),
    }
}
