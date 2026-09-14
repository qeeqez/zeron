//! Agent panel operations: cancel, expand, stop-all, clear-finished, plus
//! the plumbing that opens/closes a panel row for a chat's backend turn.
//! Standalone task agents (the panel's input row) live in `agents_task`.

use std::rc::Rc;

use gpui_kit::*;

use crate::model::{Agent, AgentStatus, Chat, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

impl Workspace {
    pub fn cancel_agent(&mut self, id: u64, cx: &mut Context<Self>) {
        // Chat-run rows hold no handles — the task and child live on the
        // Chat. `stop_chat_reply` snapshots the turn's tool calls onto the
        // row before clearing the link, then stops the backend.
        if let Some(chat_id) = self.chats.iter().find(|c| c.run_agent == Some(id)).map(|c| c.id) {
            self.stop_chat_reply(chat_id, cx);
            return;
        }
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) else { return };
        if agent.status != AgentStatus::Running {
            return;
        }
        if let Some(task) = agent.task.take() {
            drop(task); // non-detached Task cancels on drop
        }
        drop(agent.stream.take()); // dropping the stream kills the turn
        agent.status = AgentStatus::Cancelled;
        agent.step = "cancelled".into();
        settle_tools(agent);
        cx.notify();
    }

    pub fn toggle_agent_expand(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
            agent.expanded = !agent.expanded;
        }
        cx.notify();
    }

    /// Expand/collapse one tool row's output inside an agent card.
    pub fn toggle_agent_tool(&mut self, id: u64, tool_ix: usize, cx: &mut Context<Self>) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id)
            && !agent.expanded_tools.remove(&tool_ix)
        {
            agent.expanded_tools.insert(tool_ix);
        }
        cx.notify();
    }

    pub fn stop_all_agents(&mut self, cx: &mut Context<Self>) {
        // Chat-run rows first — their handles live on the Chat.
        let chat_ids: Vec<u64> = self
            .chats
            .iter()
            .filter(|c| {
                c.run_agent
                    .is_some_and(|id| self.agents.iter().any(|a| a.id == id && a.status == AgentStatus::Running))
            })
            .map(|c| c.id)
            .collect();
        for id in chat_ids {
            self.stop_chat_reply(id, cx);
        }
        for agent in &mut self.agents {
            if agent.status == AgentStatus::Running {
                agent.status = AgentStatus::Cancelled;
                agent.step = "cancelled".into();
            }
            drop(agent.task.take()); // non-detached Task cancels on drop
            drop(agent.stream.take()); // dropping the stream kills the turn
            settle_tools(agent);
        }
        cx.notify();
    }

    pub fn clear_finished_agents(&mut self, cx: &mut Context<Self>) {
        self.agents.retain(|a| a.status == AgentStatus::Running);
        cx.notify();
    }

    /// Open a panel row for a real backend turn. `steps_total` stays 0 —
    /// the backend doesn't announce its plan; `steps_done` counts tool
    /// calls as they arrive so the row shows real progress.
    pub(crate) fn spawn_run_agent(&mut self, spec: RunAgentSpec<'_>, cx: &mut Context<Self>) {
        let id = self.next_agent_id;
        self.next_agent_id += 1;
        let mut agent = Agent::new(id, spec.name, spec.lane, 0);
        agent.step = "running".into();
        self.agents.push(agent);
        if let Some(chat) = self.chats.iter_mut().find(|c| c.id == spec.chat_id) {
            chat.run_agent = Some(id);
        }
        cx.notify();
    }

    /// Record a real event on the turn's agent row. `count_step` marks
    /// tool calls — the only discrete unit a real turn exposes. The call
    /// itself renders as a tool row (derived from the chat's Tool
    /// messages), so step entries update `step` but don't duplicate into
    /// the log.
    pub(crate) fn agent_log(&mut self, chat_id: u64, entry: AgentLogEntry, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let Some(id) = chat.run_agent else { return };
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id && a.status == AgentStatus::Running) else {
            return;
        };
        if entry.count_step {
            agent.steps_done += 1;
            agent.steps_total = agent.steps_done;
            agent.step = entry.line.into();
        } else {
            agent.log.push(format!("[{}s] {}", agent.elapsed_secs, entry.line).into());
        }
        cx.notify();
    }

    /// Close the turn's agent row. No-op when the row is already closed
    /// (cancel beat the event stream to it). Tool messages still marked
    /// Running are settled — a turn that ends without a ToolCallEnd
    /// shouldn't leave a spinner behind. The chat's tool calls are
    /// snapshotted onto the agent so the row keeps them after the link
    /// drops (and after later turns relink the chat).
    pub(crate) fn finish_run_agent(&mut self, chat_id: u64, ok: bool, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        let Some(id) = chat.run_agent else { return };
        let status = if ok { ToolStatus::Done } else { ToolStatus::Failed };
        // Settle only this turn's tools — messages after the last user
        // message, same scope as `turn_tools`. Sweeping the whole history
        // would rewrite a still-Running tool from an earlier cancelled
        // turn with this turn's outcome.
        let start = turn_start(chat);
        for msg in Rc::make_mut(&mut chat.messages)[start..].iter_mut() {
            if let MessageKind::Tool(t) = &mut msg.kind
                && t.status == ToolStatus::Running
            {
                t.status = status;
            }
        }
        let tools: Vec<ToolCall> = turn_tools(chat).into_iter().cloned().collect();
        chat.run_agent = None;
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) else { return };
        agent.tools = tools;
        if agent.status == AgentStatus::Running {
            agent.status = if ok { AgentStatus::Done } else { AgentStatus::Failed };
            agent.step = "finished".into();
            agent.log.push(format!("[{}s] {}", agent.elapsed_secs, agent.status).into());
        }
        settle_tools(agent);
        cx.notify();
    }

    /// Copy the chat's current-turn tool calls onto its agent row. Must run
    /// while `chat.run_agent` still points at the row.
    pub(crate) fn snapshot_chat_tools(&mut self, chat_id: u64) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let Some(id) = chat.run_agent else { return };
        let tools: Vec<ToolCall> = turn_tools(chat).into_iter().cloned().collect();
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
            agent.tools = tools;
        }
    }
}

/// Tool rows shown under an agent card. A running chat turn derives live
/// status and output from the chat's Tool messages; everything else reads
/// the row's own snapshot (task agents record theirs from the stream).
pub(crate) fn agent_tools<'a>(ws: &'a Workspace, agent: &'a Agent) -> Vec<&'a ToolCall> {
    match ws.chats.iter().find(|c| c.run_agent == Some(agent.id)) {
        Some(chat) => turn_tools(chat),
        None => agent.tools.iter().collect(),
    }
}

/// Index where the current turn begins: just after the last user message.
fn turn_start(chat: &Chat) -> usize {
    chat.messages.iter().rposition(|m| m.role == Role::User).map_or(0, |i| i + 1)
}

/// The current turn's tool calls: Tool messages after the last user
/// message. Scoping matters — a chat's earlier turns would otherwise leak
/// their tool calls into the live row.
fn turn_tools(chat: &Chat) -> Vec<&ToolCall> {
    chat.messages[turn_start(chat)..]
        .iter()
        .filter_map(|m| match &m.kind {
            MessageKind::Tool(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// Close tool rows still marked Running once the turn is over: Done when
/// the turn finished cleanly, Failed otherwise (cancel counts — the call
/// never produced a result).
pub(crate) fn settle_tools(agent: &mut Agent) {
    let status = if agent.status == AgentStatus::Done { ToolStatus::Done } else { ToolStatus::Failed };
    for tool in &mut agent.tools {
        if tool.status == ToolStatus::Running {
            tool.status = status;
        }
    }
}

/// "42s" → "42s", "75s" → "1m 15s", "3700s" → "1h 1m" — the compact form
/// Codex uses for turn durations.
pub(crate) fn fmt_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, secs % 3600 / 60)
    }
}

/// Identity of a real backend turn for the agents panel — bundled so the
/// spawn helper stays under the argument-count lint.
pub(crate) struct RunAgentSpec<'a> {
    pub chat_id: u64,
    pub name: &'a str,
    pub lane: &'a str,
}

/// One log line for a turn's agent row; `count_step` marks tool calls.
pub(crate) struct AgentLogEntry {
    pub line: String,
    pub count_step: bool,
}

#[cfg(test)]
mod tests {
    use crate::model::{Agent, AgentStatus, ToolStatus};

    use super::settle_tools;

    fn running_tool() -> crate::model::ToolCall {
        crate::model::ToolCall {
            tool_ix: 0,
            name: "bash".into(),
            detail: "ls".into(),
            output: String::new().into(),
            status: ToolStatus::Running,
            expanded: false,
        }
    }

    #[test]
    fn settle_marks_running_tools_done_on_success() {
        let mut agent = Agent::new(1, "t", "sim", 0);
        agent.status = AgentStatus::Done;
        agent.tools.push(running_tool());
        settle_tools(&mut agent);
        assert_eq!(agent.tools[0].status, ToolStatus::Done);
    }

    #[test]
    fn settle_marks_running_tools_failed_on_cancel() {
        let mut agent = Agent::new(1, "t", "sim", 0);
        agent.status = AgentStatus::Cancelled;
        agent.tools.push(running_tool());
        settle_tools(&mut agent);
        assert_eq!(agent.tools[0].status, ToolStatus::Failed);
    }
}
