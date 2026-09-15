//! Standalone task agents: background backend turns spawned from the
//! agents panel's input row. Unlike chat-turn rows they own their event
//! stream, so they record structured tool calls (status + output) on the
//! agent itself.

use gpui_kit::*;

use crate::backend::AgentEvent;
use crate::model::{Agent, AgentStatus, ToolCall, ToolStatus};
use crate::workspace::Workspace;

impl Workspace {
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
        if self.model.is_empty() {
            // No synthetic "default" — a provider with no catalog can't run.
            agent.log.push("error: the selected provider has no models".into());
            agent.status = AgentStatus::Failed;
            self.agents.push(agent);
            cx.notify();
            return;
        }
        // Task agents aren't tied to a chat — they run in the project root
        // with the workspace's current access mode.
        let ctx = crate::backend::TurnContext::at(self.project.root().to_path_buf(), self.access);
        let stream = self.backend.send(&prompt, self.model.as_ref(), self.mode.as_ref(), &ctx);
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

    /// Pull pending events off a task agent's stream into its tool rows
    /// and log. When the stream ends (Done/Error/disconnect) the row
    /// closes and the polling task is dropped.
    fn drain_task_agent(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id && a.status == AgentStatus::Running) else {
            return;
        };
        // Collect first: applying events needs `&mut agent`, which
        // conflicts with the stream borrow while it's live.
        let (events, disconnected) = agent.stream.as_mut().map_or((Vec::new(), false), drain_stream);
        let mut closed = disconnected;
        for ev in events {
            if let Some(status) = apply_task_event(agent, &ev) {
                agent.status = status;
                agent.step = "finished".into();
                closed = true;
            }
        }
        if disconnected {
            close_disconnected(agent);
        }
        if closed {
            crate::agents::settle_tools(agent);
            agent.stream = None;
            agent.task = None; // drops this polling task at the next await
        }
        cx.notify();
    }
}

/// Drain every pending event off a stream; the second tuple field is set
/// when the producer disconnected (stream over without a terminal event).
fn drain_stream(stream: &mut crate::backend::ReplyStream) -> (Vec<AgentEvent>, bool) {
    let mut events = Vec::new();
    loop {
        match stream.events.try_recv() {
            Ok(ev) => events.push(ev),
            Err(std::sync::mpsc::TryRecvError::Empty) => return (events, false),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return (events, true),
        }
    }
}

/// Apply one backend event to a task agent's row: tool calls become
/// structured rows with live status and streamed output; diffs, usage and
/// errors append to the event log. Text deltas are skipped — the panel
/// shows activity, not prose. Returns the terminal status when the event
/// ends the turn.
fn apply_task_event(agent: &mut Agent, ev: &AgentEvent) -> Option<AgentStatus> {
    match ev {
        AgentEvent::ToolCallStart { ix, name, detail } => {
            agent.steps_done += 1;
            agent.steps_total = agent.steps_done;
            agent.step = format!("{name} {detail}").into();
            agent.tools.push(ToolCall {
                tool_ix: *ix,
                name: name.clone(),
                detail: detail.clone(),
                output: SharedString::default(),
                status: ToolStatus::Running,
                expanded: false,
            });
            None
        },
        AgentEvent::ToolCallDelta { ix, output } => {
            if let Some(tool) = agent.tools.iter_mut().find(|t| t.tool_ix == *ix) {
                tool.output = format!("{}{}", tool.output, output).into();
            }
            None
        },
        // Snapshot-style updates (turn plan checklist) replace output.
        AgentEvent::ToolCallSet { ix, output } => {
            if let Some(tool) = agent.tools.iter_mut().find(|t| t.tool_ix == *ix) {
                tool.output = output.clone();
            }
            None
        },
        AgentEvent::ToolCallEnd { ix, ok } => {
            if let Some(tool) = agent.tools.iter_mut().find(|t| t.tool_ix == *ix) {
                tool.status = if *ok { ToolStatus::Done } else { ToolStatus::Failed };
            }
            None
        },
        AgentEvent::Diff { path, added, removed, .. } => {
            agent.log.push(format!("[{}s] diff {path} +{added} -{removed}", agent.elapsed_secs).into());
            None
        },
        AgentEvent::Usage { input, output } => {
            agent.log.push(format!("[{}s] usage {input}→{output}", agent.elapsed_secs).into());
            None
        },
        AgentEvent::Done => Some(AgentStatus::Done),
        AgentEvent::Error(msg) => {
            agent.log.push(format!("[{}s] error: {msg}", agent.elapsed_secs).into());
            Some(AgentStatus::Failed)
        },
        AgentEvent::TextStart | AgentEvent::TextDelta(_) => None,
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
    use crate::backend::AgentEvent;
    use crate::model::{Agent, AgentStatus, ToolStatus};

    use super::{apply_task_event, close_disconnected};

    fn tool_start(ix: usize) -> AgentEvent {
        AgentEvent::ToolCallStart { ix, name: "bash".into(), detail: "ls".into() }
    }

    #[test]
    fn tool_call_lifecycle_tracks_status_and_output() {
        let mut agent = Agent::new(1, "t", "sim", 0);
        assert!(apply_task_event(&mut agent, &tool_start(0)).is_none());
        assert_eq!(agent.tools.len(), 1);
        assert_eq!(agent.tools[0].status, ToolStatus::Running);
        assert_eq!(agent.steps_done, 1);

        apply_task_event(&mut agent, &AgentEvent::ToolCallDelta { ix: 0, output: "out".into() });
        apply_task_event(&mut agent, &AgentEvent::ToolCallDelta { ix: 0, output: "put".into() });
        assert_eq!(agent.tools[0].output.as_str(), "output");

        assert!(apply_task_event(&mut agent, &AgentEvent::ToolCallEnd { ix: 0, ok: false }).is_none());
        assert_eq!(agent.tools[0].status, ToolStatus::Failed);
    }

    #[test]
    fn failed_tool_call_is_not_terminal() {
        let mut agent = Agent::new(1, "t", "sim", 0);
        apply_task_event(&mut agent, &tool_start(0));
        assert!(apply_task_event(&mut agent, &AgentEvent::ToolCallEnd { ix: 0, ok: false }).is_none());
        assert_eq!(agent.status, AgentStatus::Running);
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
