//! Agent panel operations: cancel, expand, stop-all, clear-finished.

use gpui_kit::*;

use crate::model::AgentStatus;
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
}
