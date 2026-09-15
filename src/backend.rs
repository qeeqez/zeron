use gpui_kit::SharedString;

use crate::model::PlanStep;

mod acp;
mod acp_decode;
mod acp_rpc;
mod approval;
mod appserver;
mod claude;
mod claude_parse;
mod codex;
mod codex_turn;
mod factory;
mod http;
mod models;
mod rpc;
mod sessions;
mod sim;
mod steer;

#[cfg(test)]
mod acp_mcp_tests;
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
mod instructions_tests;
#[cfg(test)]
mod sessions_tests;
pub use acp::AcpBackend;
pub use approval::{ApprovalCard, ApprovalDecision, ApprovalKind, ApprovalResponder, ApprovalRoute};
pub use claude::ClaudeCliBackend;
pub(crate) use claude::{auth_status as claude_auth_status, login as claude_login, logout as claude_logout};
pub use codex::CodexCliBackend;
pub use codex::fetch_mcp_status;
#[cfg(test)]
pub(crate) use codex::read_mcp_status;
pub(crate) use codex::{auth_status as codex_auth_status, login as codex_login, logout as codex_logout};
pub(crate) use factory::apply_env;
pub use factory::backend_for;
pub use http::HttpBackend;
pub use models::fetch_codex_models;
pub use steer::TurnHandle;
pub(crate) use steer::kill_slot;

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

    /// Whether permission prompts are answered without asking the user.
    /// Auto/FullAccess auto-approve; the "ask" modes surface an
    /// `AgentEvent::ApprovalRequest` card instead.
    pub fn auto_allows(self) -> bool {
        matches!(self, AccessMode::Auto | AccessMode::FullAccess)
    }

    /// How the backend answers an approval request in this mode: prompt
    /// the user, or reply immediately with a fixed decision.
    pub fn approval_route(self) -> ApprovalRoute {
        if self.auto_allows() { ApprovalRoute::Auto(ApprovalDecision::Approve) } else { ApprovalRoute::Ask }
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
    /// Reasoning effort for the turn — `None` lets the backend apply the
    /// model's own default (codex's `defaultReasoningEffort`).
    pub effort: Option<String>,
    /// Image attachment paths for this turn — sent as image inputs where
    /// the backend supports them (codex `localImage`, ACP `resource_link`),
    /// else carried by the prompt's `[Attached files:]` list.
    pub images: Vec<std::path::PathBuf>,
    /// Merged custom instructions (global setting + project file) for this
    /// turn — `None` when neither source has content. Each backend maps it
    /// onto its own system channel (codex `developerInstructions`, claude
    /// `--append-system-prompt`, ACP/HTTP a prompt prefix).
    pub instructions: Option<String>,
}

impl TurnContext {
    /// A turn rooted at `cwd` with `access`, starting a fresh thread.
    pub fn at(cwd: std::path::PathBuf, access: AccessMode) -> Self {
        Self {
            cwd,
            access,
            thread_id: None,
            effort: None,
            images: Vec::new(),
            instructions: None,
        }
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
    /// arrive as full snapshots, not deltas.
    ToolCallSet { ix: usize, output: SharedString },
    /// Tool call finished; `ok` flips status to Done/Failed.
    ToolCallEnd { ix: usize, ok: bool },
    /// The agent's plan checklist — a full snapshot that replaces the plan
    /// card keyed by `ix` (created on first sight).
    Plan { ix: usize, steps: Vec<PlanStep> },
    /// The backend needs the user's decision before this tool call can
    /// proceed — the pump thread blocks on `respond`'s pair until the UI
    /// answers (or the card's responder drops, which answers Deny).
    /// `ix` hashes the request's item/call id, matching tool-card keys.
    ApprovalRequest {
        ix: usize,
        kind: ApprovalKind,
        /// What will run — the command line, patch summary, or the ACP
        /// tool call's title.
        detail: SharedString,
        respond: ApprovalResponder,
    },
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
    /// Live handle for the in-flight turn — shared with the chat so
    /// stop/delete can kill a hung process without waiting for the pump,
    /// and so a steer can write into the turn's stdin when the backend
    /// supports it (`TurnHandle::can_steer`).
    pub child: Option<std::sync::Arc<dyn TurnHandle>>,
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

impl ReplyStream {
    /// Inject `text` into the running turn (codex `turn/steer`). False when
    /// the backend has no mid-turn input or the turn already ended — the
    /// caller queues the message instead.
    pub fn steer(&self, text: &str) -> bool {
        self.child.as_ref().is_some_and(|h| h.steer(text))
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
    /// Whether this backend can inject user text into a running turn
    /// (codex `turn/steer`). When false, steering falls back to queueing.
    fn supports_steer(&self) -> bool {
        false
    }
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
pub use sim::SimBackend;
