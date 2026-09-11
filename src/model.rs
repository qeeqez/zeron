use std::time::{Instant, SystemTime};

use gpui_kit::{SharedString, Task};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone)]
pub struct ToolCall {
    pub name: SharedString,
    pub detail: SharedString,
    pub output: SharedString,
    pub status: ToolStatus,
    pub expanded: bool,
}
#[derive(Clone)]
pub struct DiffCard {
    pub path: SharedString,
    pub added: usize,
    pub removed: usize,
    pub hunks: SharedString,
    pub expanded: bool,
    pub applied: Option<bool>,
}

#[derive(Clone)]
pub enum MessageKind {
    Text(SharedString),
    Tool(ToolCall),
    Diff(DiffCard),
}

#[derive(Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub kind: MessageKind,
    pub rating: Option<bool>,
    pub at: SystemTime,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
            reply_task: None,
            unread: false,
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

pub struct Agent {
    pub name: SharedString,
    pub lane: SharedString,
    pub status: AgentStatus,
    pub step: SharedString,
    pub steps_done: usize,
    pub steps_total: usize,
    pub elapsed_secs: u64,
    pub task: Option<Task<()>>,
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
        }
    }
}
