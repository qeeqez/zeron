/// Review types — split into `model_review.rs` for the SLOC cap.
#[path = "model_review.rs"]
mod review;
pub(crate) use review::{ReviewComment, ReviewTarget};

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
    /// The chat's one pinned message — the banner under the titlebar
    /// links back to it. Missing in files written before pinning existed.
    #[serde(default)]
    pub pinned: bool,
    /// Earlier versions of this reply, newest first — a regenerate/retry
    /// moves the outgoing reply here instead of dropping it, and the
    /// footer's `< N/M >` pager swaps one back in. Missing in files
    /// written before versioning existed; empty = no alternatives.
    #[serde(default)]
    pub alternatives: Vec<ChatMessage>,
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

    /// A failed turn's row: the backend writes errors as assistant text
    /// starting `**Error:**` (see `apply_event`), and the auth/no-model
    /// bail-outs push the same marker as a note. Drives the row's Retry
    /// affordance — `failed_flag` alone misses the note path.
    pub fn is_error(&self) -> bool {
        self.role == Role::Assistant && matches!(&self.kind, MessageKind::Text(t) if t.starts_with("**Error:**"))
    }

    /// The version chain's slot for this message: `alternatives` holds the
    /// other versions newest-first, so the position is the count of
    /// alternatives newer than `self` plus one (1-based for the pager).
    pub fn version_position(&self) -> usize {
        self.alternatives.iter().filter(|a| a.at > self.at).count() + 1
    }

    /// Swap the live message with an adjacent version: `older` steps back
    /// (the alternative just after this message's slot), `!older` steps
    /// forward to the newest. The outgoing message re-enters the chain at
    /// the vacated slot so positions stay stable across paging.
    pub fn cycle_alternative(&mut self, older: bool) {
        let pos = self.version_position();
        let alt_ix = if older {
            if pos > self.alternatives.len() {
                return; // already the oldest version
            }
            pos - 1
        } else {
            if pos <= 1 {
                return; // already the newest version
            }
            pos - 2
        };
        let alt = self.alternatives.remove(alt_ix);
        // The outgoing message carries the chain; the incoming one stored
        // none (see `Chat::adopt_alternatives`). Move the chain onto the
        // new live message, then park the outgoing one at the vacated slot.
        let mut outgoing = std::mem::replace(self, alt);
        self.alternatives = std::mem::take(&mut outgoing.alternatives);
        self.alternatives.insert(alt_ix, outgoing);
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
    /// Manual sidebar position — `0` means "unset: sort by `created_at`".
    /// Drag-reorder rewrites it (see `crate::chat_reorder`); persisted so a
    /// custom order survives restarts.
    pub order: i64,
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
    /// Unsent composer text — stashed on chat switch, written through on
    /// every keystroke, and persisted so a draft survives restarts.
    pub draft: String,
    pub unread: bool,
    /// The title was auto-generated from the first exchange (see
    /// `crate::chat_title`). Persisted so a manual rename — or a second
    /// turn after a generated title — never triggers another generation.
    pub title_generated: bool,
    /// The user named this chat themselves (rename committed) — persisted
    /// so auto-titling never overwrites it, even when the chosen name
    /// happens to match the placeholder.
    pub title_custom: bool,
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
    /// Version chain a regenerate/retry saved for the reply it replaced —
    /// the new turn's first assistant text message adopts it (see
    /// `adopt_alternatives`). Runtime state: a turn that never produces a
    /// reply leaves the chain to the next turn's first text bubble.
    pub pending_alternatives: Vec<ChatMessage>,
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
    /// The ref a worktree chat's Changes panel diffs against — `None`
    /// follows the default (the merge-base of the worktree's HEAD and the
    /// project's HEAD). Persisted; a deleted ref falls back to the default.
    pub diff_base: Option<String>,
    /// Temporary chat — never written to disk (`persist::save_chats` skips
    /// it) and gone when the chat closes or the app exits. Runtime-only:
    /// nothing ephemeral ever reaches `StoredChat`.
    pub ephemeral: bool,
    /// Backend thread this chat continues — bound by the first turn's
    /// `AgentEvent::ThreadBound` (or set when the chat was created by
    /// resuming a past session). Empty = the next send starts a fresh
    /// thread and binds whatever id it gets back.
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
    /// Long text messages the user expanded past the collapse clip — keyed
    /// by message index + timestamp so truncation can't reopen a different
    /// message. Runtime state, not persisted; long messages start collapsed.
    pub expanded_msgs: std::collections::HashSet<(usize, SystemTime)>,
    /// Per-chat spend cap in USD — overrides the global
    /// `Settings.budget_alert_usd` default; `None` rides the default.
    /// Persisted; ephemeral chats never reach disk.
    pub budget_alert_usd: Option<f64>,
    /// The cap the budget alert last fired under — compared against the
    /// *current* effective cap so raising/lowering it re-arms the alert.
    /// Runtime state, not persisted.
    pub budget_alerted: Option<f64>,
    /// The user dismissed the current alert — the banner stays hidden
    /// until the cap changes. Runtime state, not persisted.
    pub budget_dismissed: bool,
    /// `(slot, recover_interrupted)` when the transcript still lives only
    /// on disk: `load_chats` parses metadata only and `ensure_messages`
    /// re-reads the file on first open. Runtime state — never persisted.
    pub pending_load: Option<(usize, bool)>,
}

impl Chat {
    pub fn new(id: u64, title: impl Into<SharedString>) -> Self {
        Self {
            id,
            title: title.into(),
            messages: std::rc::Rc::new(Vec::new()),
            running: false,
            failed_flag: false,
            pending_alternatives: Vec::new(),
            pinned: false,
            folder: String::new(),
            created_at: SystemTime::now(),
            order: 0,
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
            title_custom: false,
            draft: String::new(),
            provider: String::new(),
            model: String::new(),
            access: None,
            budget_alert_usd: None,
            budget_alerted: None,
            budget_dismissed: false,
            effort: None,
            workdir: String::new(),
            worktree: false,
            diff_base: None,
            ephemeral: false,
            thread_id: String::new(),
            usage: crate::usage::ChatUsage::default(),
            checkpoints: Vec::new(),
            feedback: Vec::new(),
            instructions: None,
            expanded_tool_groups: std::collections::HashSet::new(),
            expanded_msgs: std::collections::HashSet::new(),
            prompt_history: Vec::new(),
            pending_load: None,
        }
    }

    /// Move the pending version chain onto `msg` — called where a reply
    /// turn creates its assistant text bubble. The outgoing reply's own
    /// alternatives were flattened into the chain at truncate time, so
    /// each stored version carries an empty chain.
    pub fn adopt_alternatives(&mut self, msg: &mut ChatMessage) {
        if self.pending_alternatives.is_empty() {
            return;
        }
        msg.alternatives = std::mem::take(&mut self.pending_alternatives);
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
