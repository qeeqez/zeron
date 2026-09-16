use std::time::{Instant, SystemTime};

use gpui_kit::{SharedString, Task};

/// One selectable model in a provider's catalog — backends return these
/// from `AgentBackend::models` for the provider→model picker.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    /// Value passed to `send`'s `model` arg.
    pub id: SharedString,
    /// Display name in the picker.
    pub label: SharedString,
    /// One-line provider description; may be empty.
    pub description: SharedString,
    /// The model's `defaultReasoningEffort` — the effort the composer
    /// picker shows until the user picks another. Empty = the backend
    /// decides; the effort picker stays hidden.
    #[serde(default)]
    pub default_effort: SharedString,
    /// The model's `supportedReasoningEfforts` — the effort picker's
    /// options. Empty = the model doesn't advertise efforts.
    #[serde(default)]
    pub efforts: Vec<SharedString>,
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
            .map(|s| format!("- [{}] {}", if s.status == PlanStatus::Done { "x" } else { " " }, s.label))
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
    /// An approval prompt card — `ApprovalCard.respond` is `Some` while
    /// the backend is still waiting on the answer.
    Approval(crate::backend::ApprovalCard),
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

impl ChatMessage {
    /// The message's Markdown source — what "Copy as Markdown" writes and
    /// what a quote reproduces. Text messages already store their source;
    /// cards serialize to the same Markdown shape `export` writes.
    pub fn markdown(&self) -> String {
        match &self.kind {
            MessageKind::Text(t) => t.to_string(),
            MessageKind::Tool(t) => format!("`{} {}`\n```\n{}\n```", t.name, t.detail, t.output),
            MessageKind::Diff(d) => format!("`{}` +{} -{}\n```diff\n{}\n```", d.path, d.added, d.removed, d.hunks),
            MessageKind::Plan(p) => p.markdown(),
            MessageKind::Approval(a) => {
                let outcome = a.decision.map_or("pending", |d| d.label());
                format!("**{}:** `{}` — {}", a.kind.label(), a.detail, outcome)
            },
        }
    }
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

/// One pending review comment on a diff line — collected in the Changes
/// panel and sent to the agent as a structured review message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewComment {
    /// Project-relative file path the comment is anchored to.
    pub path: String,
    /// Line number in the file — the new side when the diff line has one,
    /// else the old side (removed lines only exist there).
    pub line: u32,
    /// `line` counts on the old side — a removed line can share its number
    /// with an added line, so the side is part of the anchor's identity.
    pub old_side: bool,
    /// The diff line's content, quoted in the review for context.
    pub code: String,
    /// The reviewer's comment text.
    pub text: String,
}

/// The diff row the comment editor is anchored to: `file_ix` indexes
/// `Workspace::changes`, `line_ix` indexes that row's `FileDiff::lines`.
/// Indices (not line numbers) so the editor tracks its row across re-renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewTarget {
    pub file_ix: usize,
    pub line_ix: usize,
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
    /// Folder this chat is filed under in the sidebar — empty means
    /// "Unfiled". Persisted; folders exist only as long as a chat names
    /// them, so renaming/deleting a folder rewrites every member chat.
    pub folder: String,
    pub created_at: SystemTime,
    pub started_at: Option<Instant>,
    /// Wall-clock duration of the last completed turn — drives the
    /// "Worked for Ns" label under the final assistant message. Set by
    /// `complete_turn`; not persisted.
    pub last_turn: Option<std::time::Duration>,
    pub reply_task: Option<Task<()>>,
    /// The in-flight turn's stream (events receiver moved to the pump
    /// thread) — dropping it kills the child and sets `cancelled`, and
    /// `ReplyStream::steer` writes into the turn when the backend allows.
    pub stream: Option<crate::backend::ReplyStream>,
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
    /// Reasoning effort this thread's turns request — `None` = the
    /// model's `default_effort`. Same lifecycle as `provider`/`model`.
    pub effort: Option<String>,
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
    /// Token/context usage folded from the turn's `AgentEvent::Usage`
    /// stream — drives the composer meter. Runtime state, not persisted.
    pub usage: crate::usage::ChatUsage,
    /// Workdir snapshots taken before each backend turn, pinned to the
    /// turn's user message — the "Undo turn" affordance restores them.
    /// Persisted so revert survives restarts.
    pub checkpoints: Vec<crate::checkpoints::TurnCheckpoint>,
    /// "What went wrong" notes attached to thumbs-down ratings, pinned to
    /// each message's `at` timestamp (see `crate::feedback`). Persisted.
    pub feedback: Vec<crate::feedback::FeedbackNote>,
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
            folder: String::new(),
            created_at: SystemTime::now(),
            started_at: None,
            last_turn: None,
            reply_task: None,
            stream: None,
            run_agent: None,
            attachments: Vec::new(),
            archived: false,
            unread: false,
            draft: String::new(),
            provider: String::new(),
            model: String::new(),
            access: None,
            effort: None,
            workdir: String::new(),
            worktree: false,
            thread_id: String::new(),
            usage: crate::usage::ChatUsage::default(),
            checkpoints: Vec::new(),
            feedback: Vec::new(),
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
mod tests;
