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

use gpui_kit::assets::IconName;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{IndexPath, WindowExt, h_flex};
use gpui_kit::*;
use serde::Deserialize;

use crate::model::{Chat, ChatMessage};
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
            messages: chat.messages.clone(),
        }
    }

    /// A chat file this window never loaded.
    fn stored(file_ix: usize, stored: StoredChatFile) -> Self {
        Self {
            chat_id: None,
            file_ix,
            title: stored.title.into(),
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
    /// Index into the chat's message vec.
    pub msg_ix: usize,
    pub title: SharedString,
    pub snippet: SharedString,
    /// The message's timestamp — results sort newest-first on it.
    pub at: SystemTime,
}

/// Mirror of `persist::StoredChat` for targeted single-file reads — its
/// fields are private, so the serde shape is duplicated here and must
/// track the original field-for-field.
#[derive(Deserialize)]
struct StoredChatFile {
    v: u32,
    title: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    draft: String,
    #[serde(default = "std::time::SystemTime::now")]
    created_at: SystemTime,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    access: String,
    #[serde(default)]
    effort: String,
    #[serde(default)]
    workdir: String,
    #[serde(default)]
    worktree: bool,
    #[serde(default)]
    thread_id: String,
    #[serde(default)]
    checkpoints: Vec<crate::checkpoints::TurnCheckpoint>,
}

/// `(index, path)` pairs for every `N.json` chat file in `dir`, sorted —
/// the same naming `persist::save_chats` writes.
fn chat_files(dir: &Path) -> Vec<(usize, PathBuf)> {
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
fn read_stored(path: &Path) -> Option<StoredChatFile> {
    let stored: StoredChatFile = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (stored.v == 1).then_some(stored)
}

/// Every match for `query` across `docs`, newest message first. An empty
/// query matches nothing — the dialog shows its hint instead of flooding
/// the list with every message ever written.
pub(crate) fn search(docs: &[SearchDoc], query: &str) -> Vec<SearchHit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for doc in docs {
        let mut taken = 0;
        // Newest-first within a chat, capped — see PER_CHAT.
        for (msg_ix, m) in doc.messages.iter().enumerate().rev() {
            if taken >= PER_CHAT {
                break;
            }
            if !crate::chat_search::msg_matches(m, &query) {
                continue;
            }
            taken += 1;
            hits.push(SearchHit {
                chat_id: doc.chat_id,
                file_ix: doc.file_ix,
                msg_ix,
                title: doc.title.clone(),
                snippet: crate::chat_search::match_snippet(m, &query).into(),
                at: m.at,
            });
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.at));
    hits.truncate(MAX_HITS);
    hits
}

/// A result row: chat title over the match snippet, relative age at the
/// trailing edge — same shape as the palette's chat rows.
fn hit_item(hit: SearchHit) -> CommandItem {
    let title = hit.title.clone();
    let snippet = hit.snippet.clone();
    CommandItem::new().label(hit.title).child(move |_, cx| {
        h_flex()
            .flex_1()
            .gap_2()
            .items_center()
            .child(IconName::MessageSquare)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().whitespace_nowrap().text_ellipsis().child(title.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(snippet.clone()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::palette_items::rel_time(hit.at)),
            )
    })
}

/// The dialog's `Command` element, rebuilt on every workspace render —
/// `on_query` notifies so each keystroke re-runs `search` with the live
/// query against the docs snapshot.
fn search_command(state: &Entity<CommandState>, docs: &[SearchDoc], ws: &Entity<Workspace>, cx: &mut App) -> Command {
    let ws_confirm = ws.clone();
    let ws_query = ws.clone();
    let hits = search(docs, &state.read(cx).query(cx));
    let group = CommandGroup::new().label("Messages").items(hits.into_iter().map(hit_item));
    Command::new(state)
        .placeholder("Search all chats…")
        // Matching happens in `search`, not the component's substring filter.
        .filterable(false)
        .group(group)
        .empty(|state, _, cx| {
            let hint = if state.query(cx).trim().is_empty() { "Search messages across every chat" } else { "No matches" };
            div().py_6().w_full().text_center().text_sm().text_color(cx.theme().muted_foreground).child(hint)
        })
        .footer(|_, _, cx| crate::palette::command_footer("↵ open chat", cx))
        .on_query(move |_, _, cx| {
            ws_query.update(cx, |_, cx| cx.notify());
        })
        .on_confirm(move |path, window, cx| {
            ws_confirm.update(cx, |this, cx| this.confirm_global_hit(path, window, cx));
        })
        .on_cancel(|window, cx| window.close_dialog(cx))
}

impl Workspace {
    /// Cmd-Shift-F: search every conversation. Pressing it again (or with
    /// any dialog up) closes the dialog, like the palette.
    pub fn open_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
            return;
        }
        // Fresh query each open — the state entity persists across dialogs.
        self.global_search.update(cx, |state, cx| state.set_query("", window, cx));
        // the workspace lease, so it can't read `self`.
        let docs = self.search_docs();
        let state = self.global_search.clone();
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog.close_button(false).overlay_closable(true).child(search_command(&state, &docs, &ws, cx))
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
            if file_ix < self.chats.len() {
                continue;
            }
            if let Some(stored) = read_stored(&path) {
                docs.push(SearchDoc::stored(file_ix, stored));
            }
        }
        docs
    }

    /// Resolve a confirmed row to its hit and open it. The search re-runs
    /// so `path.row` resolves against the same ranked list the dialog
    /// showed — and against any disk state that moved since it opened.
    fn confirm_global_hit(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let query = self.global_search.read(cx).query(cx).to_string();
        let hits = search(&self.search_docs(), &query);
        if let Some(hit) = hits.get(path.row) {
            self.open_hit(hit, &query, window, cx);
        }
    }

    /// Open the hit's chat — loading it from its file when this window
    /// never did — and land on the matched message via the find bar.
    pub(crate) fn open_hit(&mut self, hit: &SearchHit, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ix = match hit.chat_id.and_then(|id| self.chat_index(id)) {
            Some(ix) => ix,
            None => {
                let Some(chat) = self.load_chat(hit.file_ix) else { return };
                self.chats.push(chat);
                self.chats.len() - 1
            },
        };
        self.select_chat(ix, window, cx);
        self.jump_to_message(query, hit.msg_ix, window, cx);
    }

    /// Load chat file `N.json` into a live `Chat` — the per-file half of
    /// `persist::load_chats`, minus the interrupted-turn recovery (a live
    /// turn in another window must not be marked failed here).
    fn load_chat(&mut self, file_ix: usize) -> Option<Chat> {
        let stored = read_stored(&self.project.chats_dir().join(format!("{file_ix}.json")))?;
        let mut chat = Chat::new(self.next_chat_id, stored.title);
        self.next_chat_id += 1;
        chat.messages = Rc::new(stored.messages);
        chat.pinned = stored.pinned;
        chat.archived = stored.archived;
        chat.draft = stored.draft;
        chat.created_at = stored.created_at;
        chat.provider = stored.provider;
        chat.model = stored.model;
        chat.access = (!stored.access.is_empty()).then(|| crate::backend::AccessMode::from_name(&stored.access));
        chat.effort = (!stored.effort.is_empty()).then_some(stored.effort);
        chat.workdir = stored.workdir;
        chat.worktree = stored.worktree;
        chat.thread_id = stored.thread_id;
        chat.checkpoints = stored.checkpoints;
        Some(chat)
    }
}
