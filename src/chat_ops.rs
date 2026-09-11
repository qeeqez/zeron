use gpui_kit::*;

use crate::model::{AgentStatus, Chat, MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    pub fn set_diff_applied(&mut self, ix: usize, applied: bool, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get_mut(ix) else { return };
        if let MessageKind::Diff(diff) = &mut msg.kind {
            diff.applied = Some(applied);
        }
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(ix..ix + 1, cx);
        });
        cx.notify();
    }

    pub fn rate_message(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if let Some(msg) = chat.messages.get_mut(ix) {
            msg.rating = if msg.rating == Some(up) { None } else { Some(up) };
        }
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(ix..ix + 1, cx);
        });
        cx.notify();
    }

    pub fn copy_message(&self, ix: usize, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let text = match &msg.kind {
            MessageKind::Text(t) => t.to_string(),
            MessageKind::Tool(t) => format!("{}: {}\n{}", t.name, t.detail, t.output),
            MessageKind::Diff(d) => format!("{} (+{} -{})\n{}", d.path, d.added, d.removed, d.hunks),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Stop the in-flight reply stream for the active chat.
    pub fn stop_reply(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if let Some(task) = chat.reply_task.take() {
            drop(task); // non-detached Task cancels on drop
        }
        chat.running = false;
        chat.started_at = None;
        cx.notify();
    }

    /// Duplicate chat `ix` (title + messages) as a new chat.
    pub fn duplicate_chat(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(src) = self.chats.get(ix) else { return };
        let mut copy = Chat::new(format!("{} (copy)", src.title));
        copy.messages = src.messages.clone();
        self.chats.push(copy);
        self.active = self.chats.len() - 1;
        let count = self.chats[self.active].messages.len();
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
    }

    pub fn cancel_agent(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(agent) = self.agents.get_mut(ix) else { return };
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

    /// Export chat `ix` as markdown to the clipboard.
    pub fn export_chat(&self, ix: usize, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        let mut out = format!("# {}\n\n", chat.title);
        for msg in &chat.messages {
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
            };
            let body = match &msg.kind {
                MessageKind::Text(t) => t.to_string(),
                MessageKind::Tool(t) => format!("`{} {}`\n```\n{}\n```", t.name, t.detail, t.output),
                MessageKind::Diff(d) => format!("`{}` +{} -{}\n```diff\n{}\n```", d.path, d.added, d.removed, d.hunks),
            };
            out.push_str(&format!("## {role}\n\n{body}\n\n"));
        }
        cx.write_to_clipboard(ClipboardItem::new_string(out));
    }
}

impl Workspace {
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

    /// Rough token estimate: chars/4 across all messages.
    pub fn token_estimate(&self) -> usize {
        self.chats
            .iter()
            .flat_map(|c| &c.messages)
            .map(|m| match &m.kind {
                MessageKind::Text(t) => t.len(),
                MessageKind::Tool(t) => t.output.len(),
                MessageKind::Diff(d) => d.hunks.len(),
            })
            .sum::<usize>()
            / 4
    }
}

impl Workspace {
    /// Delete every chat and start a fresh one.
    pub fn clear_all_chats(&mut self, cx: &mut Context<Self>) {
        self.chats.clear();
        self.new_chat(cx);
    }
}

impl Workspace {
    pub fn rename_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        self.open_rename(ix, window, cx);
    }

    pub fn export_active(&mut self, cx: &mut Context<Self>) {
        let ix = self.active;
        self.export_chat(ix, cx);
    }
}
