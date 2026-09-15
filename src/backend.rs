use gpui_kit::SharedString;

mod acp;
mod acp_decode;
mod acp_rpc;
mod appserver;
mod claude;
mod claude_parse;
mod codex;
mod http;
mod models;
mod rpc;
mod sessions;

#[cfg(test)]
mod acp_rpc_tests;
#[cfg(test)]
mod acp_tests;

#[cfg(test)]
mod appserver_tests;
#[cfg(test)]
mod appserver_turn_tests;
#[cfg(test)]
mod claude_tests;
#[cfg(test)]
mod codex_tests;
#[cfg(test)]
mod sessions_tests;

pub use acp::AcpBackend;
pub use claude::ClaudeCliBackend;
pub use codex::CodexCliBackend;
pub use http::HttpBackend;
pub use models::fetch_codex_models;

/// How much filesystem access an Agent-mode turn gets. Plan/Ask turns are
/// always read-only regardless of this setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    /// Read-only sandbox; every action needs approval — the agent can
    /// inspect and propose but not modify files on its own.
    Supervised,
    /// `workspace-write` sandbox; file edits inside the workspace are
    /// auto-accepted, anything else still asks.
    AutoAcceptEdits,
    /// `workspace-write` sandbox with approvals off — writes confined to
    /// the working directory, nothing prompts.
    #[default]
    Auto,
    /// `danger-full-access` — unsandboxed, like Codex's full access.
    FullAccess,
}

impl AccessMode {
    /// All modes, in settings-picker order (least to most permissive).
    pub const ALL: [AccessMode; 4] = [AccessMode::Supervised, AccessMode::AutoAcceptEdits, AccessMode::Auto, AccessMode::FullAccess];

    /// Stable id stored in settings.json.
    pub fn name(self) -> &'static str {
        match self {
            AccessMode::Supervised => "supervised",
            AccessMode::AutoAcceptEdits => "auto-accept-edits",
            AccessMode::Auto => "auto",
            AccessMode::FullAccess => "full-access",
        }
    }

    /// Label shown in the settings picker.
    pub fn label(self) -> &'static str {
        match self {
            AccessMode::Supervised => "Supervised",
            AccessMode::AutoAcceptEdits => "Auto-accept edits",
            AccessMode::Auto => "Auto",
            AccessMode::FullAccess => "Full access",
        }
    }

    /// `codex` sandbox value (`thread/start`'s `sandbox`, `codex exec -s`).
    pub fn sandbox_arg(self) -> &'static str {
        match self {
            AccessMode::Supervised => "read-only",
            AccessMode::AutoAcceptEdits | AccessMode::Auto => "workspace-write",
            AccessMode::FullAccess => "danger-full-access",
        }
    }

    /// `codex` `thread/start` `approvalPolicy` — "ask" modes keep the
    /// server-side prompt, the auto modes never do.
    pub fn approval_arg(self) -> &'static str {
        match self {
            AccessMode::Supervised => "on-request",
            AccessMode::AutoAcceptEdits => "on-failure",
            AccessMode::Auto | AccessMode::FullAccess => "never",
        }
    }

    /// Whether the turn may write files at all (Agent mode only — Plan/Ask
    /// are always read-only).
    pub fn writes(self) -> bool {
        !matches!(self, AccessMode::Supervised)
    }

    /// Whether writes stay confined to the working directory.
    pub fn workspace_only(self) -> bool {
        matches!(self, AccessMode::AutoAcceptEdits | AccessMode::Auto)
    }

    /// Whether permission prompts auto-approve (no approval UI exists, so
    /// "ask" modes decline instead).
    pub fn auto_allows(self) -> bool {
        matches!(self, AccessMode::Auto | AccessMode::FullAccess)
    }

    /// Parse a settings.json value; anything unknown falls back to the
    /// default so a stale or hand-edited file can't wedge the picker.
    /// Legacy names ("read-only", "workspace-write") map onto the closest
    /// new mode.
    pub fn from_name(name: &str) -> Self {
        match name {
            "read-only" => AccessMode::Supervised,
            "workspace-write" => AccessMode::Auto,
            _ => Self::ALL.iter().copied().find(|m| m.name() == name).unwrap_or_default(),
        }
    }
}

/// Per-turn context snapshotted from the chat at send time — the thread's
/// working directory (project root or its git worktree) and access mode.
/// A snapshot keeps a mid-turn settings change from altering a running
/// turn's sandbox or cwd.
#[derive(Clone, Debug)]
pub struct TurnContext {
    /// Directory the backend process spawns in.
    pub cwd: std::path::PathBuf,
    /// Filesystem access for Agent-mode turns.
    pub access: AccessMode,
    /// Backend thread to continue instead of starting a fresh one — set on
    /// chats created by resuming a past session (`thread/resume` on codex).
    /// `None` = the backend starts a new thread for this turn.
    pub thread_id: Option<String>,
}

impl TurnContext {
    /// A turn rooted at `cwd` with `access`, starting a fresh thread.
    pub fn at(cwd: std::path::PathBuf, access: AccessMode) -> Self {
        Self { cwd, access, thread_id: None }
    }
}

/// One past agent thread a backend can reopen — a row in the sidebar's
/// Resume section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// Backend thread id — passed back to `resume_session` and carried on
    /// `TurnContext::thread_id` so sends continue the thread.
    pub id: String,
    /// Display title — the thread's name or its first prompt line.
    pub title: String,
    /// Last activity as unix seconds (`thread.updatedAt`).
    pub updated: u64,
    /// Working directory the thread ran in.
    pub cwd: String,
}

/// A reopened thread: its identity plus the transcript the backend
/// returned, ready to land on a chat.
#[derive(Clone)]
pub struct ResumedSession {
    /// The resumed thread's id — bound to the chat so sends continue it.
    pub id: String,
    /// Thread title (name or preview).
    pub title: String,
    /// The thread's working directory.
    pub cwd: String,
    /// Past turns rendered as chat messages — user prompts, agent text,
    /// and completed tool cards.
    pub messages: Vec<crate::model::ChatMessage>,
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
    /// Replace the tool call's output wholesale — for items whose updates
    /// arrive as full snapshots (the turn plan checklist), not deltas.
    ToolCallSet { ix: usize, output: SharedString },
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
    pub events: std::sync::mpsc::Receiver<AgentEvent>,
    /// Backend child for the in-flight turn — shared with the chat so
    /// stop/delete can kill a hung process without waiting for the pump.
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

/// Sets the stream's `cancelled` flag when dropped. The reply task holds
/// one so stopping a chat signals the backend immediately — the pump
/// thread's own `ReplyStream` drop only fires once it wakes on an event,
/// which a silent HTTP response never sends.
pub(crate) struct CancelOnDrop(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl ReplyStream {
    /// A guard that sets `cancelled` when the reply task's future drops.
    pub(crate) fn cancel_guard(&self) -> CancelOnDrop {
        CancelOnDrop(self.cancelled.clone())
    }
}

/// Pluggable agent backend. Implementations live behind `dyn` so the UI
/// can swap transports without touching chat state.
pub trait AgentBackend: Send + Sync {
    /// Human-readable name for the status bar.
    fn name(&self) -> &'static str;
    /// Models this provider offers; empty means the picker shows nothing.
    /// Backends that learn their catalog at runtime return what they've seen.
    fn models(&self) -> Vec<crate::model::ModelInfo> {
        Vec::new()
    }
    /// Start a reply turn. `ctx` carries the thread's working directory and
    /// access mode, snapshotted at send time. The returned stream yields
    /// events until `Done`/`Error` or cancellation (drop the stream to
    /// cancel).
    fn send(&self, prompt: &str, model: &str, mode: &str, ctx: &TurnContext) -> ReplyStream;
    /// Whether this backend keeps resumable threads — gates the sidebar's
    /// Resume section. Cheap: no I/O, just capability.
    fn supports_sessions(&self) -> bool {
        false
    }
    /// Past threads this backend can reopen, newest first. `None` when the
    /// backend has no session support; `Some(vec![])` when it does but the
    /// list is empty or the fetch failed. Blocking — call off the UI thread.
    fn list_sessions(&self) -> Option<Vec<SessionInfo>> {
        None
    }
    /// Reopen a past thread: returns its transcript for display. `None`
    /// when unsupported or the resume failed — the caller still binds the
    /// thread id so the next send continues it. Blocking.
    fn resume_session(&self, _thread_id: &str) -> Option<ResumedSession> {
        None
    }
}
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

/// Build the backend for one provider instance — `command`/`key_env` carry
/// the kind-specific connection fields (acp spawn command, http endpoint).
pub fn backend_for(p: &crate::providers::ProviderInstance) -> std::sync::Arc<dyn AgentBackend> {
    use crate::providers::ProviderKind;
    match p.kind {
        ProviderKind::CodexCli => std::sync::Arc::new(CodexCliBackend::new()),
        ProviderKind::ClaudeCli => std::sync::Arc::new(ClaudeCliBackend::new()),
        ProviderKind::Acp => std::sync::Arc::new(AcpBackend::new(p.command.clone())),
        ProviderKind::Http => std::sync::Arc::new(HttpBackend::new(p.command.clone(), p.key_env.clone())),
        ProviderKind::Sim => std::sync::Arc::new(SimBackend),
    }
}
