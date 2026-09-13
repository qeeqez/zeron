use std::time::{Instant, SystemTime};

use gpui_kit::{SharedString, Task};

/// Model ids offered in the picker and `/model`.
pub const MODELS: [&str; 4] = ["default", "gpt-5-codex", "gpt-5", "gpt-5-mini"];

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
}

/// Token counts from a completed backend turn.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    User,
    Assistant,
}

pub struct Chat {
    /// Stable identity — positions shift on delete, so reply tasks must
    /// not capture indices.
    pub id: u64,
    pub title: SharedString,
    pub messages: Vec<ChatMessage>,
    pub running: bool,
    pub failed_flag: bool,
    pub pinned: bool,
    pub created_at: SystemTime,
    pub started_at: Option<Instant>,
    pub reply_task: Option<Task<()>>,
    /// Backend child slot for the in-flight turn — lets stop/delete kill a
    /// hung process without waiting for the pump thread.
    pub child: Option<std::sync::Arc<parking_lot::Mutex<Option<std::process::Child>>>>,
    pub draft: String,
    pub unread: bool,
    pub archived: bool,
    pub attachments: Vec<SharedString>,
}

impl Chat {
    pub fn new(id: u64, title: impl Into<SharedString>) -> Self {
        Self {
            id,
            title: title.into(),
            messages: Vec::new(),
            running: false,
            failed_flag: false,
            pinned: false,
            created_at: SystemTime::now(),
            started_at: None,
            reply_task: None,
            child: None,
            attachments: Vec::new(),
            archived: false,
            unread: false,
            draft: String::new(),
        }
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        if let Some(slot) = &self.child {
            crate::backend::kill_slot(slot);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    pub steps_done: usize,
    pub steps_total: usize,
    pub elapsed_secs: u64,
    pub task: Option<Task<()>>,
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
            steps_done: 0,
            steps_total,
            elapsed_secs: 0,
            task: None,
            log: Vec::new(),
            expanded: false,
        }
    }
}
