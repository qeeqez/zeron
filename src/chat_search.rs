use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind};
use crate::workspace::Workspace;

/// The text a chat-search query matches against for one message.
pub(crate) fn msg_matches(m: &ChatMessage, query: &str) -> bool {
    let haystacks: &[&str] = match &m.kind {
        MessageKind::Text(t) => &[t.as_str()],
        MessageKind::Tool(t) => &[t.name.as_str(), t.detail.as_str(), t.output.as_str()],
        MessageKind::Diff(d) => &[d.path.as_str(), d.hunks.as_str()],
    };
    haystacks.iter().any(|h| h.to_lowercase().contains(query))
}

/// Should a newly pushed last message grow the scroller? False only when a
/// non-empty search query is active and the message doesn't match.
pub(crate) fn grows_scroller(is_active: bool, msg: &ChatMessage, query: &str) -> bool {
    is_active && (query.is_empty() || msg_matches(msg, query))
}

/// Scroller position of the last message: its vec index, or the match count
/// minus one when a search query filters the list.
pub(crate) fn last_scroller_pos(messages: &[ChatMessage], query: &str) -> usize {
    if query.is_empty() {
        messages.len().saturating_sub(1)
    } else {
        messages.iter().filter(|m| msg_matches(m, query)).count().saturating_sub(1)
    }
}

impl Workspace {
    /// Number of messages visible under the current search query — all when
    /// search is closed or the query is empty.
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

    /// Scroller position of message `real_ix` — its vec index, or the count
    /// of matching messages before it when a search query filters the list.
    pub(crate) fn filtered_pos(&self, real_ix: usize, cx: &App) -> usize {
        let query = self.chat_search.read(cx).value().to_lowercase();
        if !self.chat_search_open || query.is_empty() {
            return real_ix;
        }
        // Clamp — a stale ix from a truncated chat would panic the slice.
        let end = real_ix.min(self.chats[self.active].messages.len());
        self.chats[self.active].messages[..end].iter().filter(|m| msg_matches(m, &query)).count()
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
pub(crate) fn focus_new_chat(composer: &Entity<gpui_kit::component::input::TextareaState>, window: &mut Window, cx: &mut App) {
    composer.update(cx, |s, cx| {
        s.set_value("", window, cx);
        s.focus(window, cx);
    });
    window.set_window_title("New chat — Rixl Code");
}
