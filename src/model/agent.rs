//! Agents-panel row model — split from `model.rs` for the SLOC cap. Reached
//! as `crate::model::{Agent, AgentStatus}` via the re-export there.

use gpui_kit::{SharedString, Task};

use super::ToolCall;

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
