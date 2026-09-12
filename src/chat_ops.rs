use gpui_kit::*;

use crate::model::{Chat, MessageKind, Role};
use crate::workspace::Workspace;
impl Workspace {
    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        self.chats.push(Chat::new(id, "New chat"));
        self.active = self.chats.len() - 1;
        self.recall_ix = None;
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
        let composer = self.composer.clone();
        cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |_this, window, cx| {
                composer.update(cx, |s, cx| s.focus(window, cx));
                window.set_window_title("Rixl Code — New chat");
            });
        })
        .detach();
        cx.notify();
        self.save();
    }

    pub fn select_chat(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.chats.len() || index == self.active {
            return;
        }
        // Save current draft, restore target's.
        self.chats[self.active].draft = self.composer.read(cx).value().to_string();
        self.active = index;
        self.recall_ix = None;
        self.chats[index].unread = false;
        let draft = self.chats[index].draft.clone();
        self.composer.update(cx, |s, cx| {
            s.set_value(draft, window, cx);
            s.focus(window, cx);
        });
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        window.set_window_title(&format!("Rixl Code — {}", self.chats[index].title));
        cx.notify();
        self.save();
        self.save_settings();
    }
}

impl Workspace {
    /// Stop the in-flight reply stream for the active chat. Dropping the
    /// task drops the ReplyStream, which kills the backend child process.
    pub fn stop_reply(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if let Some(task) = chat.reply_task.take() {
            drop(task); // non-detached Task cancels on drop
        }
        chat.running = false;
        chat.started_at = None;
        cx.notify();
        self.save();
    }

    /// Duplicate chat `ix` (title + messages) as a new chat and select it.
    pub fn duplicate_chat(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.chats.get(ix) else { return };
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        let mut copy = Chat::new(id, format!("{} (copy)", src.title));
        copy.messages = src.messages.clone();
        copy.draft = src.draft.clone();
        self.chats.push(copy);
        let new_ix = self.chats.len() - 1;
        self.select_chat(new_ix, window, cx);
    }

    pub fn toggle_archive(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(ix) {
            chat.archived = !chat.archived;
        }
        // If we archived the active chat, switch to the first non-archived.
        if self.chats[self.active].archived {
            if let Some(next) = self.chats.iter().position(|c| !c.archived) {
                self.select_chat(next, window, cx);
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
        let attachments = &mut self.chats[self.active].attachments;
        if ix < attachments.len() {
            attachments.remove(ix);
        }
        cx.notify();
    }

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
