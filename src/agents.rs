//! Agent panel operations: cancel, expand, stop-all, clear-finished.

use gpui_kit::*;

use crate::model::{Agent, AgentStatus};
use crate::workspace::Workspace;

impl Workspace {
    pub fn cancel_agent(&mut self, id: u64, cx: &mut Context<Self>) {
        // Chat-run rows hold no handles — the task and child live on the
        // Chat. Resolve the link and stop that reply so the backend
        // process actually dies.
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
        cx.notify();
    }

    pub fn toggle_agent_expand(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
            agent.expanded = !agent.expanded;
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
            if agent.status != AgentStatus::Running {
                continue;
            }
            if let Some(task) = agent.task.take() {
                drop(task);
            }
            drop(agent.stream.take());
            agent.status = AgentStatus::Cancelled;
            agent.step = "cancelled".into();
        }
        cx.notify();
    }

    pub fn clear_finished_agents(&mut self, cx: &mut Context<Self>) {
        self.agents.retain(|a| a.status == AgentStatus::Running);
        cx.notify();
    }

    /// Spawn a standalone background task: a real backend turn that
    /// reports into the panel without tying up a chat. The agent owns the
    /// stream; cancel/stop-all drop it to kill the turn.
    pub fn spawn_task_agent(&mut self, prompt: String, cx: &mut Context<Self>) {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        let id = self.next_agent_id;
        self.next_agent_id += 1;
        let name = if prompt.chars().count() > 24 {
            format!("{}…", prompt.chars().take(24).collect::<String>())
        } else {
            prompt.clone()
        };
        let mut agent = Agent::new(id, name, self.backend.name(), 0);
        agent.step = "running".into();
        agent.log.push("[0s] task started".into());
        let stream = self.backend.send(&prompt, self.model.as_ref(), self.mode.as_ref());
        agent.stream = Some(stream);
        self.agents.push(agent);
        cx.notify();
        let task = cx.spawn(async move |this, cx| {
            // Poll the stream's channel; the pump inside the backend
            // already runs on its own thread.
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
                let _ = this.update(cx, |this, cx| this.drain_task_agent(id, cx));
            }
        });
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
            agent.task = Some(task);
        }
    }

    /// Pull pending events off a task agent's stream into its log. When
    /// the stream ends (Done/Error/disconnect) the row closes and the
    /// polling task is dropped.
    fn drain_task_agent(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id && a.status == AgentStatus::Running) else {
            return;
        };
        let Some(stream) = &mut agent.stream else { return };
        let mut closed = false;
        loop {
            let ev = match stream.events.try_recv() {
                Ok(ev) => ev,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    close_disconnected(agent);
                    closed = true;
                    break;
                },
            };
            let (line, is_step, terminal) = task_event_line(&ev);
            if is_step {
                agent.steps_done += 1;
                agent.steps_total = agent.steps_done;
            }
            match (line, is_step) {
                (Some(line), true) => {
                    agent.step = line.clone().into();
                    agent.log.push(format!("[{}s] {line}", agent.elapsed_secs).into());
                },
                (Some(line), false) => agent.log.push(format!("[{}s] {line}", agent.elapsed_secs).into()),
                (None, _) => {},
            }
            if let Some(status) = terminal {
                agent.status = status;
                agent.step = "finished".into();
                closed = true;
            }
        }
        if closed {
            agent.stream = None;
            agent.task = None; // drops this polling task at the next await
        }
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

/// Map one backend event to (log line, counts-as-step, terminal status).
/// Text deltas are skipped — the panel shows activity, not prose.
/// A failed tool call is item-level, not turn-level: it logs but does not
/// close the row — the turn may recover and continue.
fn task_event_line(ev: &crate::backend::AgentEvent) -> (Option<String>, bool, Option<AgentStatus>) {
    use crate::backend::AgentEvent as E;
    match ev {
        E::ToolCallStart { name, detail, .. } => (Some(format!("{name} {detail}")), true, None),
        E::ToolCallEnd { ok, .. } => ((!ok).then(|| "tool call failed".to_string()), false, None),
        E::Diff { path, added, removed, .. } => (Some(format!("diff {path} +{added} -{removed}")), false, None),
        E::Usage { input, output } => (Some(format!("usage {input}→{output}")), false, None),
        E::Done => (None, false, Some(AgentStatus::Done)),
        E::Error(msg) => (Some(format!("error: {msg}")), false, Some(AgentStatus::Failed)),
        E::TextStart | E::TextDelta(_) | E::ToolCallDelta { .. } => (None, false, None),
    }
}

/// Stream ended without a terminal event — mark the row Done, unless an
/// earlier Error already closed it as Failed (a producer that errors then
/// drops its sender lands here).
fn close_disconnected(agent: &mut Agent) {
    if agent.status == AgentStatus::Running {
        agent.status = AgentStatus::Done;
    }
    agent.step = "finished".into();
}

#[cfg(test)]
mod tests {
    use crate::model::{Agent, AgentStatus};

    use super::{close_disconnected, task_event_line};

    #[test]
    fn failed_tool_call_is_not_terminal() {
        let (line, is_step, terminal) = task_event_line(&crate::backend::AgentEvent::ToolCallEnd { ix: 0, ok: false });
        assert!(line.is_some());
        assert!(!is_step);
        assert!(terminal.is_none());
    }

    #[test]
    fn disconnect_preserves_failed_status() {
        let mut agent = Agent::new(1, "t", "sim", 0);
        agent.status = AgentStatus::Failed;
        close_disconnected(&mut agent);
        assert_eq!(agent.status, AgentStatus::Failed);
    }

    #[test]
    fn disconnect_closes_running_as_done() {
        let mut agent = Agent::new(1, "t", "sim", 0);
        close_disconnected(&mut agent);
        assert_eq!(agent.status, AgentStatus::Done);
    }
}
