use gpui_kit::*;

use crate::model::{Chat, MessageKind};
use crate::workspace::Workspace;
impl Workspace {
    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        // Stash the current draft before switching — the composer text
        // belongs to the outgoing chat. `get_mut`: first launch has no chats.
        if let Some(chat) = self.chats.get_mut(self.active) {
            chat.draft = self.composer.read(cx).value().to_string();
        }
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        self.chats.push(Chat::new(id, "New chat"));
        self.active = self.chats.len() - 1;
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
        let composer = self.composer.clone();
        cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |_this, window, cx| crate::chat_search::focus_new_chat(&composer, window, cx));
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
        self.recall_saved = None;
        self.search_match_ix = 0;
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
        window.set_window_title(&format!("{} — Rixl Code", self.chats[index].title));
        cx.notify();
        self.save();
        self.save_settings();
    }
}

impl Workspace {
    /// Current vec index of the chat with `id` — positions shift on delete,
    /// so UI closures must capture the id and resolve at action time.
    pub(crate) fn chat_index(&self, id: u64) -> Option<usize> {
        self.chats.iter().position(|c| c.id == id)
    }
}

impl Workspace {
    /// Stop the in-flight reply stream for the active chat. Kills the
    /// backend child directly — a hung child would otherwise leak because
    /// the pump thread only drops the stream when it wakes on an event.
    pub fn stop_reply(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if let Some(slot) = chat.child.take() {
            crate::backend::kill_slot(&slot);
        }
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
}

impl Workspace {
    pub fn toggle_backend(&mut self, cx: &mut Context<Self>) {
        self.backend = if matches!(self.backend.name(), "codex-cli") {
            std::sync::Arc::new(crate::backend::SimBackend)
        } else {
            std::sync::Arc::new(crate::backend::CodexCliBackend::new())
        };
        self.save_settings();
        cx.notify();
    }

    pub fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(index) {
            chat.pinned = !chat.pinned;
        }
        cx.notify();
        self.save();
    }

    pub fn delete_chat(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats.len() <= 1 || index >= self.chats.len() {
            return;
        }
        let title = self.chats[index].title.clone();
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            &format!("Delete “{title}”?"),
            Some("This cannot be undone."),
            &[gpui_kit::PromptButton::ok("Delete"), gpui_kit::PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.delete_chat_now(index, window, cx));
        })
        .detach();
    }

    fn delete_chat_now(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Re-check: the prompt is async — chats may have shrunk meanwhile.
        if self.chats.len() <= 1 || index >= self.chats.len() {
            return;
        }
        let was_active = index == self.active;
        // Chat::drop kills the child slot and cancels the reply task.
        self.chats.remove(index);
        if self.active >= self.chats.len() {
            self.active = self.chats.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        self.recall_ix = None;
        self.recall_saved = None;
        if was_active {
            // Composer still holds the deleted chat's draft — restore the
            // newly-active chat's draft instead.
            let draft = self.chats[self.active].draft.clone();
            self.composer.update(cx, |s, cx| {
                s.set_value(draft, window, cx);
            });
        }
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        self.save();
    }
}

impl Workspace {
    /// Rough token estimate: chars/4 across the active chat's messages.
    pub fn token_estimate(&self) -> usize {
        self.chats[self.active]
            .messages
            .iter()
            .map(|m| match &m.kind {
                MessageKind::Text(t) => t.len(),
                MessageKind::Tool(t) => t.output.len(),
                MessageKind::Diff(d) => d.hunks.len(),
            })
            .sum::<usize>()
            / 4
    }

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
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.save_settings();
        cx.notify();
    }

    pub fn toggle_agents_panel(&mut self, cx: &mut Context<Self>) {
        self.agents_panel_open = !self.agents_panel_open;
        cx.notify();
    }

    pub fn running_agents(&self) -> usize {
        self.agents.iter().filter(|a| a.status == crate::model::AgentStatus::Running).count()
    }
}
