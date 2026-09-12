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
    pub rating: Option<bool>,
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
    pub title: SharedString,
    pub messages: Vec<ChatMessage>,
    pub running: bool,
    pub failed_flag: bool,
    pub pinned: bool,
    pub created_at: SystemTime,
    pub started_at: Option<Instant>,
    pub reply_task: Option<Task<()>>,
    pub draft: String,
    pub unread: bool,
    pub archived: bool,
    pub attachments: Vec<SharedString>,
}

impl Chat {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            messages: Vec::new(),
            running: false,
            failed_flag: false,
            pinned: false,
            created_at: SystemTime::now(),
            started_at: None,
            draft: String::new(),
            unread: false,
            archived: false,
            reply_task: None,
            attachments: Vec::new(),
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
    pub fn new(name: impl Into<SharedString>, lane: impl Into<SharedString>, steps_total: usize) -> Self {
        Self {
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
