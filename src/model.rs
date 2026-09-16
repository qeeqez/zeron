use std::time::{Instant, SystemTime};

use gpui_kit::{SharedString, Task};

mod agent;
mod color;
pub use agent::{Agent, AgentStatus};
pub use color::ChatColor;

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
    /// Starred for the chat ⋯ menu's Bookmarks list — missing in files
    /// written before bookmarks existed.
    #[serde(default)]
    pub bookmarked: bool,
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
    /// Color tag for visual grouping — `None` renders no dot. Persisted by
    /// name; ephemeral chats never reach disk so their tag is runtime-only.
    pub color: Option<ChatColor>,
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
    /// The title was auto-generated from the first exchange (see
    /// `crate::chat_title`). Persisted so a manual rename — or a second
    /// turn after a generated title — never triggers another generation.
    pub title_generated: bool,
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
    /// Composer prompt history — every accepted send, oldest first, capped
    /// (see `crate::send::composer_history`). Persisted so Up-recall survives
    /// restarts; missing in files written before history existed.
    pub prompt_history: Vec<String>,
    /// The thread's working directory — the project root, or its git
    /// worktree when `default_workspace` is Worktree. Empty = project root.
    pub workdir: String,
    /// `workdir` is a git worktree owned by this thread — removed when the
    /// chat is deleted.
    pub worktree: bool,
    /// Temporary chat — never written to disk (`persist::save_chats` skips
    /// it) and gone when the chat closes or the app exits. Runtime-only:
    /// nothing ephemeral ever reaches `StoredChat`.
    pub ephemeral: bool,
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
    /// Per-chat custom instructions — appended to the turn's merged
    /// instructions after the global setting and project file (see
    /// `crate::instructions::for_chat_turn`). `None` = no override.
    /// Persisted; ephemeral chats never reach disk.
    pub instructions: Option<String>,
    /// Tool-call groups the user expanded — keyed by the run's head message
    /// index + timestamp so truncation can't reopen a different group.
    /// Runtime state, not persisted; groups start collapsed.
    pub expanded_tool_groups: std::collections::HashSet<(usize, SystemTime)>,
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
            color: None,
            last_turn: None,
            reply_task: None,
            stream: None,
            run_agent: None,
            attachments: Vec::new(),
            archived: false,
            unread: false,
            title_generated: false,
            draft: String::new(),
            provider: String::new(),
            model: String::new(),
            access: None,
            effort: None,
            workdir: String::new(),
            worktree: false,
            ephemeral: false,
            thread_id: String::new(),
            usage: crate::usage::ChatUsage::default(),
            checkpoints: Vec::new(),
            feedback: Vec::new(),
            instructions: None,
            expanded_tool_groups: std::collections::HashSet::new(),
            prompt_history: Vec::new(),
        }
    }

    /// Record how long the just-finished turn took and clear `started_at`.
    /// Callers: `finish_reply`, `finish_stream` (simulate.rs),
    /// `stop_reply` — each replaces its `chat.started_at = None` with this.
    pub fn complete_turn(&mut self) {
        self.last_turn = self.started_at.take().map(|t| t.elapsed());
    }
}

#[cfg(test)]
mod tests;
