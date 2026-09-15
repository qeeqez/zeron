use std::time::{Instant, SystemTime};

use gpui_kit::{SharedString, Task};

/// Codex model ids the picker falls back to when the app-server catalog
/// can't be fetched (codex missing, offline, error).
pub const CODEX_FALLBACK: [&str; 3] = ["gpt-5-codex", "gpt-5", "gpt-5-mini"];

/// One selectable model in a provider's catalog — backends return these
/// from `AgentBackend::models` for the provider→model picker.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    /// Value passed to `send`'s `model` arg.
    pub id: SharedString,
    /// Display name in the picker.
    pub label: SharedString,
    /// One-line provider description; may be empty.
    pub description: SharedString,
}

/// Codex's static catalog — used until `model/list` lands and whenever the
/// fetch fails.
pub fn codex_fallback_models() -> Vec<ModelInfo> {
    CODEX_FALLBACK
        .iter()
        .map(|id| ModelInfo {
            id: (*id).into(),
            label: (*id).into(),
            description: SharedString::default(),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    /// Backend-assigned index — parallel tool calls interleave, so deltas
    /// must match on this, not "last tool message".
    pub tool_ix: usize,
    pub name: SharedString,
    pub detail: SharedString,
    pub output: SharedString,
    pub status: ToolStatus,
    pub expanded: bool,
}

/// One checklist step's progress — mirrors codex's `update_plan` statuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Done,
}

/// One step of the agent's plan checklist.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanStep {
    /// Position in the checklist — backends don't carry stable step ids.
    pub id: usize,
    pub label: SharedString,
    pub status: PlanStatus,
}

/// The agent's live plan checklist — replaced wholesale on each update.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanCard {
    /// Backend-assigned index, same routing role as `ToolCall::tool_ix`.
    pub plan_ix: usize,
    pub steps: Vec<PlanStep>,
}

impl PlanCard {
    /// Checklist as a markdown task list — used by copy and export.
    pub fn markdown(&self) -> String {
        self.steps
            .iter()
            .map(|s| {
                let mark = if s.status == PlanStatus::Done { "x" } else { " " };
                format!("- [{mark}] {}", s.label)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DiffCard {
    pub path: SharedString,
    pub added: usize,
    pub removed: usize,
    pub hunks: SharedString,
    pub expanded: bool,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum MessageKind {
    Text(SharedString),
    Tool(ToolCall),
    Diff(DiffCard),
    Plan(PlanCard),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub kind: MessageKind,
    /// Missing in early v1 files.
    #[serde(default)]
    pub rating: Option<bool>,
    /// Missing in early v1 files — fall back to now().
    #[serde(default = "std::time::SystemTime::now")]
    pub at: SystemTime,
    /// Token usage reported by the backend for this reply.
    #[serde(default)]
    pub usage: Option<Usage>,
    /// Files attached to this message — preserved for retry.
    #[serde(default)]
    pub attachments: Vec<SharedString>,
}

/// Token counts from a completed backend turn.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    User,
    Assistant,
}

pub struct Chat {
    /// Shared with the scroller's render closure — `Rc::make_mut` clones
    /// only when a snapshot is still alive.
    pub messages: std::rc::Rc<Vec<ChatMessage>>,
    pub id: u64,
    pub title: SharedString,
    pub running: bool,
    pub failed_flag: bool,
    pub pinned: bool,
    pub created_at: SystemTime,
    pub started_at: Option<Instant>,
    /// Wall-clock duration of the last completed turn — drives the
    /// "Worked for Ns" label under the final assistant message. Set by
    /// `complete_turn`; not persisted.
    pub last_turn: Option<std::time::Duration>,
    pub reply_task: Option<Task<()>>,
    /// Backend child slot for the in-flight turn — lets stop/delete kill a
    /// hung process without waiting for the pump thread.
    pub child: Option<std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>>,
    /// Agents-panel row tracking the in-flight turn — lets stop/finish
    /// close the row without scanning by name.
    pub run_agent: Option<u64>,
    pub draft: String,
    pub unread: bool,
    pub archived: bool,
    pub attachments: Vec<SharedString>,
    /// Provider instance id this thread sends on — stamped at creation from
    /// `default_model` (or the live selection) and restored into the
    /// workspace on select. Empty = legacy chat: follow the selection.
    pub provider: String,
    /// Model id within `provider`'s catalog — same lifecycle as `provider`.
    pub model: String,
    /// Filesystem access for this thread's turns; `None` = legacy chat,
    /// follow the workspace setting.
    pub access: Option<crate::backend::AccessMode>,
    /// The thread's working directory — the project root, or its git
    /// worktree when `default_workspace` is Worktree. Empty = project root.
    pub workdir: String,
    /// `workdir` is a git worktree owned by this thread — removed when the
    /// chat is deleted.
    pub worktree: bool,
    /// Backend thread this chat continues — set when the chat was created
    /// by resuming a past codex session. Empty = each send starts a fresh
    /// thread.
    pub thread_id: String,
}

impl Chat {
    pub fn new(id: u64, title: impl Into<SharedString>) -> Self {
        Self {
            id,
            title: title.into(),
            messages: std::rc::Rc::new(Vec::new()),
            running: false,
            failed_flag: false,
            pinned: false,
            created_at: SystemTime::now(),
            started_at: None,
            last_turn: None,
            reply_task: None,
            child: None,
            run_agent: None,
            attachments: Vec::new(),
            archived: false,
            unread: false,
            draft: String::new(),
            provider: String::new(),
            model: String::new(),
            access: None,
            workdir: String::new(),
            worktree: false,
            thread_id: String::new(),
        }
    }

    /// Record how long the just-finished turn took and clear `started_at`.
    /// Callers: `finish_reply` (backend_run.rs), `finish_stream`
    /// (simulate.rs), `stop_reply` (chat_ops.rs) — each replaces its
    /// `chat.started_at = None` with this.
    pub fn complete_turn(&mut self) {
        self.last_turn = self.started_at.take().map(|t| t.elapsed());
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        if let Some(slot) = &self.child {
            crate::backend::kill_slot(slot);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        f.write_str(s)
    }
}

pub struct Agent {
    /// Stable identity — positions shift when finished agents are cleared.
    pub id: u64,
    pub name: SharedString,
    pub lane: SharedString,
    pub status: AgentStatus,
    pub step: SharedString,
    pub task: Option<Task<()>>,
    /// Live backend turn for task agents — dropping it cancels the turn.
    /// `None` for simulated agents and chat-turn rows (those are owned by
    /// the chat's reply_task/child).
    pub stream: Option<crate::backend::ReplyStream>,
    /// Tool calls this turn ran — populated for task agents from the event
    /// stream; chat-turn rows read the chat's Tool messages instead.
    pub tools: Vec<ToolCall>,
    /// Panel-side expansion for tool rows, keyed by `tool_ix` — kept off
    /// `ToolCall.expanded` so chat-derived rows don't share state with the
    /// message surface.
    pub expanded_tools: std::collections::HashSet<usize>,
    pub steps_done: usize,
    pub steps_total: usize,
    /// The backend announced a plan — `steps_done`/`steps_total` track the
    /// checklist, so tool calls no longer drive the counters.
    pub has_plan: bool,
    pub elapsed_secs: u64,
    pub log: Vec<SharedString>,
    pub expanded: bool,
}

impl Agent {
    pub fn new(id: u64, name: impl Into<SharedString>, lane: impl Into<SharedString>, steps_total: usize) -> Self {
        Self {
            id,
            name: name.into(),
            lane: lane.into(),
            status: AgentStatus::Running,
            step: "starting".into(),
            task: None,
            stream: None,
            tools: Vec::new(),
            expanded_tools: std::collections::HashSet::new(),
            steps_done: 0,
            steps_total,
            has_plan: false,
            elapsed_secs: 0,
            log: Vec::new(),
            expanded: false,
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        drop(self.task.take()); // non-detached Task cancels on drop
        drop(self.stream.take()); // dropping the stream kills the turn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_turn_records_duration() {
        let mut chat = Chat::new(1, "t");
        chat.started_at = Some(Instant::now() - std::time::Duration::from_secs(3));
        chat.complete_turn();
        assert!(chat.started_at.is_none());
        assert!(chat.last_turn.is_some_and(|d| d.as_secs() >= 3));
    }

    #[test]
    fn complete_turn_without_start_records_nothing() {
        let mut chat = Chat::new(1, "t");
        chat.complete_turn();
        assert!(chat.last_turn.is_none());
    }
}
