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
}

impl Chat {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            messages: Vec::new(),
            running: false,
            failed_flag: false,
        }
    }
}
