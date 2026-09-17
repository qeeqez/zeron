use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// The text fields a query matches against for one message — the same
/// haystacks drive in-chat search, find, global search and its snippets.
pub(crate) fn haystacks(m: &ChatMessage) -> Vec<&str> {
    match &m.kind {
        MessageKind::Text(t) => vec![t.as_str()],
        MessageKind::Tool(t) => vec![t.name.as_str(), t.detail.as_str(), t.output.as_str()],
        MessageKind::Diff(d) => vec![d.path.as_str(), d.hunks.as_str()],
        MessageKind::Plan(p) => p.steps.iter().map(|s| s.label.as_str()).collect(),
        MessageKind::Approval(a) => vec![a.kind.label(), a.detail.as_str()],
    }
}

/// The text a chat-search query matches against for one message.
/// `query` must already be lowercase.
pub(crate) fn msg_matches(m: &ChatMessage, query: &str) -> bool {
    haystacks(m).iter().any(|h| h.to_lowercase().contains(query))
}

/// A one-line excerpt of the first field containing `query` (lowercase),
/// centered on the match — the result row's snippet in global search.
pub(crate) fn match_snippet(m: &ChatMessage, query: &str) -> String {
    let Some(hay) = haystacks(m).into_iter().find(|h| h.to_lowercase().contains(query)) else {
        return String::new();
    };
    let start = hay.to_lowercase().find(query).unwrap_or(0);
    // Snap to char boundaries — lowercasing can shift byte offsets.
    let mut start = start.min(hay.len());
    while start > 0 && !hay.is_char_boundary(start) {
        start -= 1;
    }
    const CTX: usize = 40;
    let mut from = start.saturating_sub(CTX);
    while from > 0 && !hay.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (start + query.len() + CTX).min(hay.len());
    while to < hay.len() && !hay.is_char_boundary(to) {
        to += 1;
    }
    let body: String = hay[from..to].split_whitespace().collect::<Vec<_>>().join(" ");
    let prefix = if from > 0 { "…" } else { "" };
    let suffix = if to < hay.len() { "…" } else { "" };
    format!("{prefix}{body}{suffix}")
}

/// One line of surrounding context for a search hit: the neighboring
/// message's role plus its text, whitespace-collapsed and clipped to ~72
/// chars. The neighbor is whichever side carries the conversational
/// anchor — a user hit shows the reply it drew, an assistant hit shows
/// the prompt it answered; at the transcript's edge the other side is
/// used. `None` when the hit is the only message or the neighbor has no
/// text (e.g. a bare approval card).
pub(crate) fn context_line(messages: &[ChatMessage], msg_ix: usize) -> Option<(Role, SharedString)> {
    let m = messages.get(msg_ix)?;
    let (prev, next) = (msg_ix.checked_sub(1), msg_ix.checked_add(1));
    let order = match m.role {
        Role::User => [next, prev],
        Role::Assistant => [prev, next],
    };
    let (role, text) = order.into_iter().flatten().find_map(|ix| {
        let n = messages.get(ix)?;
        let text = haystacks(n).join(" ");
        (!text.trim().is_empty()).then_some((n.role, text))
    })?;
    const MAX: usize = 72;
    let body: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped = if body.len() > MAX {
        let mut end = MAX;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &body[..end])
    } else {
        body
    };
    Some((role, clipped.into()))
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

    /// Scroll the scroller to the row showing message `real_ix`, accounting
    /// for the chat-search filter — the shared jump used by find, global
    /// search and message navigation. A collapsed target expands first so a
    /// match inside the clipped region is visible on landing.
    pub(crate) fn scroll_to_message(&mut self, real_ix: usize, cx: &mut Context<Self>) {
        self.expand_msg(real_ix, cx);
        let pos = self.filtered_pos(real_ix, cx);
        self.scroller.update(cx, |s, cx| s.scroll_to_item(pos, cx));
    }
}

impl Workspace {
    /// The pill's click: jump to the live edge and mark the chat read.
    /// `scroll_to_end` rather than `scroll_to_message` — it also resumes
    /// tail-following, so the transcript stays pinned as replies stream.
    pub(crate) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.pill_anchor = None;
        self.chats[self.active].unread = false;
        self.mark_chat_activity_read(self.chats[self.active].created_at);
        crate::dock_badge::update(cx);
        self.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
        cx.notify();
    }
}

/// Refresh the pill's anchor from the scroller's tail state — called from
/// `render_chat`, which re-runs on every scroller `cx.notify`. `anchor`
/// snapshots the visible-row count when the transcript leaves the tail so
/// the pill can count rows appended while scrolled up; returning to the
/// tail clears it. `MessageScrollerState` exposes `is_scrolled_up` but no
/// visible range, so "new" counts arrivals since the scroll-away, not rows
/// below the fold.
///
/// Detach is sticky across turns: scrolling up pauses tail-follow and only
/// the user re-engages it — wheeling back to the bottom, clicking the pill,
/// or a `reset` (chat switch, send, search). A turn ending does not snap
/// the view back; the transcript stays where the user left it.
pub(crate) fn update_pill_anchor(scroller: &Entity<MessageScrollerState>, anchor: &mut Option<usize>, visible_count: usize, cx: &App) {
    if scroller.read(cx).is_scrolled_up() {
        if anchor.is_none() {
            *anchor = Some(visible_count);
        }
    } else {
        *anchor = None;
    }
}

/// The scroller's built-in jump button as the "↓ N new" pill — the kit
/// owns its bottom-center overlay, fade transition and auto-hide; this
/// renderer only supplies the label and the click. `unseen` is the count
/// of rows appended since the transcript left the tail.
pub(crate) fn pill_renderer(ws: Entity<Workspace>, unseen: usize) -> impl FnOnce(Button) -> Button + 'static {
    let label = if unseen > 0 { format!("{unseen} new") } else { "Latest".to_string() };
    move |button| {
        button.label(label).on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| this.jump_to_latest(cx));
        })
    }
}

impl Workspace {
    pub fn open_chat_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_search_open = !self.chat_search_open;
        self.search_match_ix = 0;
        // The find bar and the filter share the strip under the titlebar —
        // never show both. Closing the bar resets its role filter too.
        self.find.open = false;
        self.find.role = role_filter::RoleFilter::All;
        if self.chat_search_open {
            let input = self.chat_search.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        } else {
            self.chat_search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    }

    /// Number of messages matching the chat-search query.
    fn match_count(&self, cx: &App) -> usize {
        let query = self.chat_search.read(cx).value().to_lowercase();
        if query.is_empty() {
            return 0;
        }
        self.chats[self.active].messages.iter().filter(|m| msg_matches(m, &query)).count()
    }

    /// Enter in chat search: jump to next match; Shift+Enter: previous.
    /// `search_match_ix` is the position within the filtered list, which is
    /// what the scroller indexes. A collapsed match expands first so the
    /// hit inside the clipped region is visible on landing.
    pub fn jump_to_match(&mut self, back: bool, cx: &mut Context<Self>) {
        let query = self.chat_search.read(cx).value().to_lowercase();
        let matches = self.match_count(cx);
        if matches == 0 {
            return;
        }
        self.search_match_ix = if back {
            self.search_match_ix.checked_sub(1).unwrap_or(matches - 1)
        } else {
            (self.search_match_ix + 1) % matches
        };
        // The filtered position maps back to the message's real index.
        if let Some((real_ix, _)) = self.chats[self.active]
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| msg_matches(m, &query))
            .nth(self.search_match_ix)
        {
            self.expand_msg(real_ix, cx);
        }
        self.scroller.update(cx, |s, cx| {
            s.scroll_to_item(self.search_match_ix, cx);
        });
        cx.notify();
    }

    /// The search bar under the titlebar — filters the transcript to
    /// matching messages. The close button reuses `open_chat_search`, which
    /// also clears the query and resets the scroller.
    pub(crate) fn chat_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(IconName::Search)
            .child(div().flex_1().child(Input::new(&self.chat_search).appearance(true)))
            .child(
                div()
                    .id("close-search")
                    .cursor_pointer()
                    .text_color(cx.theme().muted_foreground)
                    .child(IconName::X)
                    .on_click(cx.listener(|this, _, window, cx| this.open_chat_search(window, cx))),
            )
    }
}

impl Workspace {
    /// Sidebar recency bucket: 0 pinned, 1 today, 2 last 7 days, 3 older.
    pub(crate) fn chat_bucket(&self, ix: usize) -> usize {
        let Some(chat) = self.chats.get(ix) else { return 3 };
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
    /// buckets, newest first within each. A drag-reordered chat's `order`
    /// overrides `created_at` as the within-bucket key (see
    /// `crate::workspace::sort_order`). Archived chats are excluded and
    /// `query` filters by title. Cmd+1..9 resolves against this order so
    /// the shortcut matches what the sidebar shows.
    pub(crate) fn sidebar_order(&self, query: &str) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.chats.len())
            .filter(|ix| !self.chats[*ix].archived && (query.is_empty() || self.chats[*ix].title.to_lowercase().contains(query)))
            .collect();
        order.sort_by_key(|ix| (self.chat_bucket(*ix), std::cmp::Reverse(crate::workspace::sort_order(&self.chats[*ix]))));
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

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[path = "role_filter.rs"]
pub(crate) mod role_filter;

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "scroll_pill_tests.rs"]
mod scroll_pill_tests;
