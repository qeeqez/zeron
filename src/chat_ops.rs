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
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
        let composer = self.composer.clone();
        cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |_this, window, cx| focus_new_chat(&composer, window, cx));
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

    pub(crate) fn filtered_count(&self, cx: &App) -> usize {
        let query = self.chat_search.read(cx).value().to_string().to_lowercase();
        if !self.chat_search_open || query.is_empty() {
            return self.chats[self.active].messages.len();
        }
        self.chats[self.active].messages.iter().filter(|m| msg_matches(m, &query)).count()
    }

    /// Whether a just-appended last message should grow the scroller count —
    /// false when chat search is open and the message doesn't match.
    pub(crate) fn push_visible(&self, cx: &App) -> bool {
        let query = self.chat_search.read(cx).value().to_string().to_lowercase();
        if !self.chat_search_open || query.is_empty() {
            return true;
        }
        let Some(m) = self.chats[self.active].messages.last() else { return false };
        msg_matches(m, &query)
    }
}

/// The text a chat-search query matches against for one message.
pub(crate) fn msg_matches(m: &crate::model::ChatMessage, query: &str) -> bool {
    let text = match &m.kind {
        MessageKind::Text(t) => t.as_str(),
        MessageKind::Tool(t) => t.name.as_str(),
        MessageKind::Diff(d) => d.path.as_str(),
    };
    text.to_lowercase().contains(query)
}

/// Should a newly pushed last message grow the scroller? False only when a
/// non-empty search query is active and the message doesn't match.
pub(crate) fn grows_scroller(is_active: bool, msg: &crate::model::ChatMessage, query: &str) -> bool {
    is_active && (query.is_empty() || msg_matches(msg, query))
}

/// Scroller position of the last message: its vec index, or the match count
/// minus one when a search query filters the list.
pub(crate) fn last_scroller_pos(messages: &[crate::model::ChatMessage], query: &str) -> usize {
    if query.is_empty() {
        messages.len().saturating_sub(1)
    } else {
        messages.iter().filter(|m| msg_matches(m, query)).count().saturating_sub(1)
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

impl Workspace {
    /// Sidebar recency bucket: 0 pinned, 1 today, 2 last 7 days, 3 older.
    pub(crate) fn chat_bucket(&self, ix: usize) -> usize {
        let chat = &self.chats[ix];
        if chat.pinned {
            return 0;
        }
        let day = std::time::Duration::from_secs(86_400);
        match std::time::SystemTime::now().duration_since(chat.created_at) {
            Ok(d) if d < day => 1,
            Ok(d) if d < day * 7 => 2,
            _ => 3,
        }
    }

    /// Chat indices in sidebar display order — pinned first, then recency
    /// buckets, newest first within each. Archived chats are excluded and
    /// `query` filters by title. Cmd+1..9 resolves against this order so
    /// the shortcut matches what the sidebar shows.
    pub(crate) fn sidebar_order(&self, query: &str) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.chats.len())
            .filter(|ix| !self.chats[*ix].archived && (query.is_empty() || self.chats[*ix].title.to_lowercase().contains(query)))
            .collect();
        order.sort_by_key(|ix| (self.chat_bucket(*ix), std::cmp::Reverse(self.chats[*ix].created_at)));
        order
    }
}

/// Focus the cleared composer and set the window title for a fresh chat.
fn focus_new_chat(composer: &Entity<gpui_kit::component::input::TextareaState>, window: &mut Window, cx: &mut App) {
    composer.update(cx, |s, cx| {
        s.set_value("", window, cx);
        s.focus(window, cx);
    });
    window.set_window_title("Rixl Code — New chat");
}
