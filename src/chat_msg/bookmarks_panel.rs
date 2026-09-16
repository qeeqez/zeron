//! The Bookmarks panel's state and cross-chat bookmark collection — split
//! from `chat_msg.rs`/`workspace.rs` for the SLOC cap and declared from
//! `chat_msg.rs` beside `toggle_bookmark` (`main.rs` is at the cap). The
//! panel itself renders in `views::bookmarks_panel`; this file owns what it
//! shows: every bookmarked message across the project's *loaded* chats,
//! grouped per chat in sidebar order.
//!
//! Scope note: only `self.chats` is scanned — chats still on disk that this
//! window never loaded don't appear until opened (or pulled in by global
//! search). Loading every chat file to find its stars would re-read the
//! whole project on each render; loaded-only is the v1 trade-off.

use std::rc::Rc;

use gpui_kit::*;

use crate::workspace::Workspace;

actions!([ToggleBookmarks]);

/// Bookmarks-panel state — just the open flag today; kept as a struct (like
/// `PlanPanel`/`SnapshotsState`) so panel state has one home.
#[derive(Default)]
pub struct BookmarksPanel {
    /// Whether the side panel is mounted — persisted as
    /// `Settings.bookmarks_panel_open`, toggled by `toggle_bookmarks_panel`.
    pub open: bool,
}

/// One bookmarked message as a panel row — ids, not references, so the list
/// survives a render cycle without borrowing the workspace.
pub struct BookmarkRow {
    /// Owning chat's id — resolved to a vec index at click time (positions
    /// shift on delete, so rows capture the id, not the index).
    pub chat_id: u64,
    /// Message index within that chat.
    pub msg_ix: usize,
    /// One-line preview — `snippet(m, 80)`.
    pub snippet: String,
    /// Message timestamp — the row's trailing relative-time label.
    pub at: std::time::SystemTime,
}

/// A chat's bookmarked messages under its title — the panel's group unit.
pub struct BookmarkGroup {
    pub chat_id: u64,
    pub title: SharedString,
    pub rows: Vec<BookmarkRow>,
}

/// One-line preview for bookmark lists — whitespace squashed, clipped at
/// `max` chars so long replies stay one row. Shared by the ⋯ submenu (60)
/// and the panel rows (80).
pub(crate) fn snippet(msg: &crate::model::ChatMessage, max: usize) -> String {
    let squashed = msg.markdown().split_whitespace().collect::<Vec<_>>().join(" ");
    // `nth(max)` is the (max+1)-th char's byte index — Some means clip.
    match squashed.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &squashed[..i]),
        None => squashed,
    }
}

impl Workspace {
    /// Every loaded chat's bookmarks, grouped in sidebar order (pinned and
    /// recency buckets first; archived chats trail in storage order).
    /// Chats without stars drop out entirely.
    pub(crate) fn bookmark_groups(&self) -> Vec<BookmarkGroup> {
        let mut order = self.sidebar_order("");
        order.extend((0..self.chats.len()).filter(|ix| self.chats[*ix].archived));
        order
            .into_iter()
            .filter_map(|ix| {
                let chat = &self.chats[ix];
                let rows: Vec<BookmarkRow> = chat
                    .messages
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.bookmarked)
                    .map(|(msg_ix, m)| BookmarkRow { chat_id: chat.id, msg_ix, snippet: snippet(m, 80), at: m.at })
                    .collect();
                (!rows.is_empty()).then(|| BookmarkGroup { chat_id: chat.id, title: chat.title.clone(), rows })
            })
            .collect()
    }

    /// Total starred messages across loaded chats — the sidebar row's suffix.
    pub(crate) fn bookmark_count(&self) -> usize {
        self.chats.iter().flat_map(|c| c.messages.iter()).filter(|m| m.bookmarked).count()
    }

    /// Toggle the Bookmarks panel; the open flag persists like the plan
    /// panel's.
    pub fn toggle_bookmarks_panel(&mut self, cx: &mut Context<Self>) {
        self.bookmarks_panel.open = !self.bookmarks_panel.open;
        self.save_settings();
        cx.notify();
    }

    /// A panel row's click: switch to the bookmark's chat and scroll the
    /// transcript to the message — the same `select_chat` +
    /// `scroll_to_message` pair global search uses.
    pub fn open_bookmark(&mut self, chat_id: u64, msg_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.chat_index(chat_id) else { return };
        self.select_chat(ix, window, cx);
        self.scroll_to_message(msg_ix, cx);
    }

    /// A row's ×: unstar that message in its own chat — `toggle_bookmark`
    /// only reaches the active chat, so the panel needs the id-addressed
    /// variant. A stale index after a transcript edit is a no-op.
    pub fn unbookmark(&mut self, chat_id: u64, msg_ix: usize, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        let Some(msg) = Rc::make_mut(&mut chat.messages).get_mut(msg_ix) else { return };
        if !msg.bookmarked {
            return;
        }
        msg.bookmarked = false;
        cx.notify();
        self.save();
    }

    /// The header's "Clear all": unstar every message in every loaded chat.
    /// One save at the end — `unbookmark` would write per row.
    pub fn clear_all_bookmarks(&mut self, cx: &mut Context<Self>) {
        let mut cleared = false;
        for chat in &mut self.chats {
            for msg in Rc::make_mut(&mut chat.messages) {
                cleared |= std::mem::take(&mut msg.bookmarked);
            }
        }
        if !cleared {
            return;
        }
        cx.notify();
        self.save();
    }
}

#[cfg(test)]
#[path = "bookmarks_panel_tests.rs"]
mod bookmarks_panel_tests;
