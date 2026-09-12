use gpui_kit::*;

use crate::model::{AgentStatus, Chat, MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    pub fn rate_message(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if let Some(msg) = chat.messages.get_mut(ix) {
            msg.rating = if msg.rating == Some(up) { None } else { Some(up) };
        }
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(ix..ix + 1, cx);
        });
        cx.notify();
        self.save();
    }

    /// Load message `ix` into the composer and truncate the chat after it,
    /// so re-sending replaces the original turn.
    pub fn edit_message(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let Some(text) = chat.messages.get(ix).and_then(|m| match &m.kind {
            MessageKind::Text(t) if m.role == Role::User => Some(t.to_string()),
            _ => None,
        }) else {
            return;
        };
        chat.messages.truncate(ix);
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        let count = self.chats[self.active].messages.len();
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
        self.save();
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
        self.backend.cancel();
        chat.running = false;
        chat.started_at = None;
        cx.notify();
        self.save();
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
        self.save();
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

    pub fn toggle_agent_expand(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(agent) = self.agents.get_mut(ix) {
            agent.expanded = !agent.expanded;
        }
        cx.notify();
    }

    pub fn toggle_archive(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(ix) {
            chat.archived = !chat.archived;
        }
        // If we archived the active chat, switch to the first non-archived.
        if self.chats[self.active].archived {
            if let Some(next) = self.chats.iter().position(|c| !c.archived) {
                self.active = next;
            } else {
                self.new_chat(cx);
            }
        }
        cx.notify();
        self.save();
    }

    /// Open the native file picker and attach the chosen files.
    pub fn attach_file(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let _ = this.update(cx, |this, cx| this.add_attachments(paths, cx));
        })
        .detach();
    }

    /// Export chat `ix` as markdown via the native save dialog.
    pub fn export_chat(&mut self, ix: usize, cx: &mut Context<Self>) {
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
        let name = format!("{}.md", chat.title.replace(['/', '\\'], "-"));
        let home = std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&home, Some(&name));
        cx.spawn(async move |_this, _cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                std::fs::write(path, out).ok();
            }
        })
        .detach();
    }

    /// Append unique file paths to the active chat's attachments.
    pub(crate) fn add_attachments(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        for name in paths.iter().map(|p| p.to_string_lossy().into_owned()) {
            if !chat.attachments.iter().any(|a| a.as_str() == name) {
                chat.attachments.push(name.into());
            }
        }
        cx.notify();
    }

    pub fn remove_attachment(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.chats[self.active].attachments.remove(ix);
        cx.notify();
    }
}

impl Workspace {
    pub(crate) fn filtered_count(&self, cx: &App) -> usize {
        let query = self.chat_search.read(cx).value().to_string().to_lowercase();
        if !self.chat_search_open || query.is_empty() {
            return self.chats[self.active].messages.len();
        }
        self.chats[self.active]
            .messages
            .iter()
            .filter(|m| {
                let text = match &m.kind {
                    MessageKind::Text(t) => t.as_str(),
                    MessageKind::Tool(t) => t.name.as_str(),
                    MessageKind::Diff(d) => d.path.as_str(),
                };
                text.to_lowercase().contains(&query)
            })
            .count()
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
    pub fn rename_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        self.open_rename(ix, window, cx);
    }

    /// Delete every chat and start a fresh one (native confirm).
    pub fn clear_all_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            "Delete all chats?",
            Some("Every conversation will be removed. This cannot be undone."),
            &[gpui_kit::PromptButton::ok("Delete All"), gpui_kit::PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.chats.clear();
                this.new_chat(cx);
            });
        })
        .detach();
    }
}

impl Workspace {
    pub fn export_active(&mut self, cx: &mut Context<Self>) {
        let ix = self.active;
        self.export_chat(ix, cx);
    }
}

impl Workspace {
    /// Copy the active chat's text messages to the clipboard.
    pub fn copy_transcript(&mut self, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let text = chat
            .messages
            .iter()
            .filter_map(|m| match &m.kind {
                MessageKind::Text(t) => Some(format!("{}: {}", if m.role == Role::User { "You" } else { "Rixl" }, t)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}
