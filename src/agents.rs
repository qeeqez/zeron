//! Agent panel operations: cancel, expand, stop-all, clear-finished.

use gpui_kit::*;

use crate::model::{Agent, AgentStatus};
use crate::workspace::Workspace;

impl Workspace {
    pub fn cancel_agent(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) else { return };
        if agent.status != AgentStatus::Running {
            return;
        }
        if let Some(task) = agent.task.take() {
            drop(task); // non-detached Task cancels on drop
        }
        agent.status = AgentStatus::Cancelled;
        agent.step = "cancelled".into();
        cx.notify();
    }

    pub fn toggle_agent_expand(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
            agent.expanded = !agent.expanded;
        }
        cx.notify();
    }

    pub fn stop_all_agents(&mut self, cx: &mut Context<Self>) {
        for agent in &mut self.agents {
            if agent.status != AgentStatus::Running {
                continue;
            }
            if let Some(task) = agent.task.take() {
                drop(task);
            }
            agent.status = AgentStatus::Cancelled;
            agent.step = "cancelled".into();
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

    /// Append a real event line to the turn's agent row and bump progress.
    /// `count_step` is set for tool calls — the only discrete unit a real
    /// turn exposes.
    pub(crate) fn agent_log(&mut self, chat_id: u64, entry: AgentLogEntry, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let Some(id) = chat.run_agent else { return };
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id && a.status == AgentStatus::Running) else {
            return;
        };
        if entry.count_step {
            agent.steps_done += 1;
            agent.steps_total = agent.steps_done;
            agent.step = entry.line.clone().into();
        }
        agent.log.push(format!("[{}s] {}", agent.elapsed_secs, entry.line).into());
        cx.notify();
    }

    /// Close the turn's agent row. No-op when the row is already closed
    /// (cancel beat the event stream to it).
    pub(crate) fn finish_run_agent(&mut self, chat_id: u64, ok: bool, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        let Some(id) = chat.run_agent.take() else { return };
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id && a.status == AgentStatus::Running) else {
            return;
        };
        agent.status = if ok { AgentStatus::Done } else { AgentStatus::Failed };
        agent.step = "finished".into();
        agent.log.push(format!("[{}s] {}", agent.elapsed_secs, agent.status).into());
        cx.notify();
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
