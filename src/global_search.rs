//! Global search (Cmd-Shift-F): a query field plus a results list of
//! (chat title, message snippet, timestamp) across every conversation —
//! loaded chats and chat files on disk this window never loaded.
//!
//! The dialog reuses the palette's `Command` component: the workspace
//! snapshots searchable docs at open time (the builder runs while `render`
//! holds the entity lease), each keystroke rebuilds the list via
//! `on_query` → `cx.notify()`, and confirming a row re-runs the same
//! search so the `IndexPath` resolves against what the user saw.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::component::{IndexPath, WindowExt};
use gpui_kit::*;

use crate::chat_search::role_filter::RoleFilter;
use crate::model::{Chat, ChatMessage, Role};
use crate::workspace::Workspace;

/// Most matches shown per chat — one busy thread shouldn't crowd out
/// every other conversation.
const PER_CHAT: usize = 3;
/// Total result cap — the list is virtualized but the scan isn't free.
const MAX_HITS: usize = 50;

/// One searchable conversation: a loaded chat (`chat_id` set) or a chat
/// file on disk this window never loaded (`file_ix` only).
#[derive(Clone)]
pub(crate) struct SearchDoc {
    /// `Workspace::chats` id — `None` for disk-only chats.
    pub chat_id: Option<u64>,
    /// The chat's `N.json` slot — loads the file when there's no live chat.
    pub file_ix: usize,
    pub title: SharedString,
    /// Provider instance id the chat sends on (`Chat.provider`) — may be
    /// empty on legacy chats; the Provider filter matches it verbatim.
    pub provider: String,
    /// Model id within the provider's catalog (`Chat.model`) — same
    /// lifecycle as `provider`.
    pub model: String,
    pub messages: Rc<Vec<ChatMessage>>,
}
impl SearchDoc {
    /// A loaded chat — `file_ix` is its position in `Workspace::chats`,
    /// which is also the slot `save_chats` writes it to.
    fn live(ix: usize, chat: &Chat) -> Self {
        Self {
            chat_id: Some(chat.id),
            file_ix: ix,
            title: chat.title.clone(),
            provider: chat.provider.clone(),
            model: chat.model.clone(),
            messages: chat.messages.clone(),
        }
    }
    fn stored(file_ix: usize, stored: crate::persist::StoredChat) -> Self {
        Self {
            chat_id: None,
            file_ix,
            title: stored.title.into(),
            provider: stored.provider,
            model: stored.model,
            messages: Rc::new(stored.messages),
        }
    }
}

/// One matched message: enough to render the row and to open the chat at
/// the match.
#[derive(Clone)]
pub(crate) struct SearchHit {
    pub chat_id: Option<u64>,
    pub file_ix: usize,
    /// Index into the chat's message vec — `open_hit` scrolls to it via
    /// the find bar.
    pub msg_ix: usize,
    pub title: SharedString,
    pub snippet: SharedString,
    /// The neighboring message's role + truncated text — the row's second
    /// line of context (see `chat_search::context_line`).
    pub context: Option<(Role, SharedString)>,
    /// The hit chat's provider/model stamps — what the Provider and Model
    /// filters matched on.
    pub provider: String,
    pub model: String,
    /// The message's timestamp — results sort newest-first on it.
    pub at: SystemTime,
}

/// The Date chip's presets — rolling windows like the sidebar's recency
/// buckets ("Today" = the last 24h), so a kept selection never goes stale
/// the way a frozen calendar boundary would.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum DateRange {
    /// No date bound — the default.
    #[default]
    Any,
    /// The last 24 hours.
    Day,
    /// The last 7 days.
    Week,
    /// The last 30 days.
    Month,
}

impl DateRange {
    /// The chip/menu label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Any => "Any date",
            Self::Day => "Today",
            Self::Week => "This week",
            Self::Month => "This month",
        }
    }

    /// The preset's lower bound, evaluated now — `None` for `Any`.
    fn cutoff(self) -> Option<SystemTime> {
        let days = match self {
            Self::Any => None,
            Self::Day => Some(1),
            Self::Week => Some(7),
            Self::Month => Some(30),
        };
        days.and_then(|d| SystemTime::now().checked_sub(std::time::Duration::from_secs(d * 86_400)))
    }
}

/// The filter row's selections, applied by `search` after the text match.
/// `None`/`All` means "no constraint" — an all-default value filters
/// nothing. Session-scoped on `Workspace::search_filters`, never persisted.
#[derive(Clone, Default)]
pub(crate) struct SearchFilters {
    /// Drop hits older than this — wins over `date` when both are set.
    pub date_from: Option<SystemTime>,
    /// Drop hits newer than this.
    pub date_to: Option<SystemTime>,
    /// The Date chip's preset — supplies the lower bound while `date_from`
    /// is unset, and is what the chip labels.
    pub date: DateRange,
    /// Drop hits whose chat's `model` differs.
    pub model: Option<String>,
    /// Drop hits whose chat's `provider` differs.
    pub provider: Option<String>,
    /// Drop hits whose message's role differs — `All` keeps both sides.
    pub role: RoleFilter,
}

/// `(index, path)` pairs for every `N.json` chat file in `dir`, sorted —
/// the same naming `persist::save_chats` writes.
pub(crate) fn chat_files(dir: &Path) -> Vec<(usize, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(usize, PathBuf)> = entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let ix = path.file_stem()?.to_str()?.parse::<usize>().ok()?;
            Some((ix, path))
        })
        .collect();
    files.sort_by_key(|(ix, _)| *ix);
    files
}

/// Parse one chat file; `None` on unreadable or foreign-format content.
pub(crate) fn read_stored(path: &Path) -> Option<crate::persist::StoredChat> {
    let stored: crate::persist::StoredChat = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (stored.v == 1).then_some(stored)
}

/// Every match for `query` across `docs`, newest message first. An empty
/// query matches nothing — the dialog shows its hint instead of flooding
/// the list with every message ever written. `filters` narrows the text
/// matches: provider/model drop whole chats, the date bound drops
/// individual messages.
pub(crate) fn search(docs: &[SearchDoc], query: &str, filters: &SearchFilters) -> Vec<SearchHit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let from = filters.date_from.or_else(|| filters.date.cutoff());
    let mut hits = Vec::new();
    for doc in docs {
        if filters.provider.as_ref().is_some_and(|p| *p != doc.provider) || filters.model.as_ref().is_some_and(|m| *m != doc.model) {
            continue;
        }
        let mut taken = 0;
        // Newest-first within a chat, capped — see PER_CHAT.
        for (msg_ix, m) in doc.messages.iter().enumerate().rev() {
            if taken >= PER_CHAT {
                break;
            }
            if !filters.role.matches(m.role) || !crate::chat_search::msg_matches(m, &query) {
                continue;
            }
            if from.is_some_and(|f| m.at < f) || filters.date_to.is_some_and(|t| m.at > t) {
                continue;
            }
            taken += 1;
            hits.push(SearchHit {
                chat_id: doc.chat_id,
                file_ix: doc.file_ix,
                msg_ix,
                title: doc.title.clone(),
                snippet: crate::chat_search::match_snippet(m, &query).into(),
                context: crate::chat_search::context_line(&doc.messages, msg_ix),
                provider: doc.provider.clone(),
                model: doc.model.clone(),
                at: m.at,
            });
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.at));
    hits.truncate(MAX_HITS);
    hits
}

impl Workspace {
    /// Cmd-Shift-F: search every conversation. Pressing it again (or with
    /// any dialog up) closes the dialog, like the palette.
    pub fn open_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_global_search_seeded("", window, cx);
    }

    /// The dialog with `query` already typed — the sidebar's "+N more" row
    /// carries its query over so the full result set is one click away.
    pub(crate) fn open_global_search_seeded(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
            return;
        }
        // The state entity persists across dialogs — set, don't append.
        self.global_search.update(cx, |state, cx| state.set_query(query, window, cx));
        // the workspace lease, so it can't read `self`.
        let docs = self.search_docs();
        let state = self.global_search.clone();
        let filters = self.search_filters.clone();
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .close_button(false)
                .overlay_closable(true)
                .child(crate::views::global_search::search_command(&state, &docs, &filters, &ws, cx))
        });
        // The dialog focuses its own handle on open; the query field needs
        // focus so typing and ↑↓/Enter reach the Command context.
        self.global_search.update(cx, |state, cx| state.focus(window, cx));
    }

    /// The searchable set: every loaded chat plus on-disk chat files beyond
    /// the loaded set (written by another window or a previous run).
    pub(crate) fn search_docs(&self) -> Vec<SearchDoc> {
        // Temporary chats are unsearchable — they never reach disk.
        let live = self.chats.iter().enumerate().filter(|x| !x.1.ephemeral);
        let mut docs: Vec<SearchDoc> = live.map(|(ix, chat)| SearchDoc::live(ix, chat)).collect();
        for (file_ix, path) in chat_files(&self.project.chats_dir()) {
            if file_ix >= self.chats.len()
                && let Some(stored) = read_stored(&path)
            {
                docs.push(SearchDoc::stored(file_ix, stored));
            }
        }
        docs
    }

    /// Resolve a confirmed row to its hit and open it. The search re-runs
    /// so `path.row` resolves against the same ranked list the dialog
    /// showed — and against any disk state that moved since it opened.
    pub(crate) fn confirm_global_hit(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let query = self.global_search.read(cx).query(cx).to_string();
        let hits = search(&self.search_docs(), &query, self.search_filters.read(cx));
        if let Some(hit) = hits.get(path.row) {
            self.open_hit(hit, &query, window, cx);
        }
    }

    /// Open the hit's chat — loading it from its file when this window
    /// never did — and land on the matched message via the find bar.
    /// `SidebarMsgHit::as_search_hit` feeds the sidebar's Messages rows
    /// through the same path.
    pub(crate) fn open_hit(&mut self, hit: &SearchHit, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ix = hit.chat_id.and_then(|id| self.chat_index(id)).or_else(|| {
            let chat = self.load_chat(hit.file_ix)?;
            self.chats.push(chat);
            Some(self.chats.len() - 1)
        });
        let Some(ix) = ix else { return };
        self.select_chat(ix, window, cx);
        self.jump_to_message(query, hit.msg_ix, window, cx);
    }

    /// Load chat file `N.json` into a live `Chat` — the per-file half of
    /// `persist::load_chats`, minus the interrupted-turn recovery (a live
    /// turn in another window must not be marked failed here).
    fn load_chat(&mut self, file_ix: usize) -> Option<Chat> {
        let stored = read_stored(&self.project.chats_dir().join(format!("{file_ix}.json")))?;
        let chat = stored.into_chat(self.next_chat_id);
        self.next_chat_id += 1;
        Some(chat)
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[path = "sidebar_search.rs"]
pub(crate) mod sidebar_search;

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "global_search_context_tests.rs"]
mod global_search_context_tests;

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "search_filter_tests.rs"]
mod search_filter_tests;

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "search_role_tests.rs"]
mod search_role_tests;
