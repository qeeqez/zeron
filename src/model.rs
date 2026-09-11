use gpui_kit::SharedString;

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
}

impl Chat {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            messages: Vec::new(),
            running: false,
            failed_flag: false,
            pinned: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone)]
pub struct Agent {
    pub name: SharedString,
    pub lane: SharedString,
    pub status: AgentStatus,
    pub step: SharedString,
    pub steps_done: usize,
    pub steps_total: usize,
    pub elapsed_secs: u64,
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
        }
    }
}
