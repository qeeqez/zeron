//! Sidebar message search — the chat-list field's second half. A non-empty
//! query still filters titles synchronously; this module adds the body
//! scan whose hits render under "Messages" (see `views::sidebar::messages`).
//!
//! Live chats scan in memory on each keystroke — cheap, same cost the
//! Cmd-Shift-F dialog pays per keystroke. Chat files this window never
//! loaded are parsed on the background executor after a debounce so typing
//! never stalls on disk; a generation counter drops stale results. Clicks
//! reuse the dialog's open-and-jump path (`open_chat_at` → find bar).

use std::time::SystemTime;

use gpui_kit::*;

use crate::model::ChatMessage;
use crate::persist::{chat_files, read_stored};
use crate::workspace::Workspace;

/// Keystroke → disk scan delay. Live chats rescan instantly; only the
/// on-disk transcripts wait out the debounce.
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
/// Most chats listed under Messages — the rest collapse into "+N more".
pub(crate) const MAX_ROWS: usize = 50;
/// Chat files larger than this are skipped — a transcript that big costs
/// more to parse than a sidebar keystroke is worth.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// One chat with at least one message-body match — the row under
/// "Messages". `msg_ix` is the newest matching message, the click target.
#[derive(Clone)]
pub(crate) struct SidebarMsgHit {
    /// `Workspace::chats` id — `None` for disk-only chats.
    pub chat_id: Option<u64>,
    /// The chat's `N.json` slot — loads the file when there's no live chat.
    pub file_ix: usize,
    /// Index into the chat's message vec — `open_chat_at` scrolls to it.
    pub msg_ix: usize,
    pub title: SharedString,
    /// Excerpt centered on the match (see `chat_search::match_snippet`).
    pub snippet: SharedString,
    /// Every matching message in the chat — the row's count badge.
    pub count: usize,
    /// The matched message's timestamp — hits sort newest-first on it.
    pub at: SystemTime,
}

/// The pending-chat identity maps the disk scan resolves file ids with —
/// `pending_load` slot hints plus `created_at`s, none of the `Rc`s a live
/// `Chat` holds (the scan runs on the background executor).
struct PendingLookup {
    /// File slot → `(chat id, created_at)` — the transcript's last known
    /// position. `created_at` verifies the hint before a live id attaches:
    /// another window may have rewritten the slots, and a foreign file
    /// must not be attributed to this chat.
    by_slot: std::collections::HashMap<usize, (u64, SystemTime)>,
    /// `created_at` → chat id, pending chats only — catches a pending
    /// transcript that drifted past the loaded set (a `None` id would
    /// load a duplicate on click).
    by_at: std::collections::HashMap<SystemTime, u64>,
    /// `created_at` of every live chat — a hydrated chat's file is already
    /// searched in the live half, so a copy drifting past the loaded set
    /// drops here instead of becoming a disk-only hit that loads a
    /// duplicate on click.
    live_ats: std::collections::HashSet<SystemTime>,
    /// `Workspace::chats.len()` at scan time — slots past it are disk-only.
    live_len: usize,
}

impl PendingLookup {
    /// A scanned file's live chat id: `Some(Some(id))` attaches the hit to
    /// a live chat, `Some(None)` treats the file as disk-only, `None`
    /// drops it (a foreign file sitting at a live index, or a hydrated
    /// chat's drifted file the live half already covered).
    fn live_id(&self, ix: usize, stored: &crate::persist::StoredChat) -> Option<Option<u64>> {
        if let Some((id, at)) = self.by_slot.get(&ix)
            && stored.created_at == Some(*at)
        {
            return Some(Some(*id));
        }
        // Stale hint or unscanned slot — `created_at` still names the
        // pending chat the file belongs to, if any.
        if let Some(id) = stored.created_at.and_then(|at| self.by_at.get(&at)) {
            return Some(Some(*id));
        }
        if stored.created_at.is_some_and(|at| self.live_ats.contains(&at)) {
            return None;
        }
        (ix >= self.live_len).then_some(None)
    }
}

/// The chat's newest body match plus its total match count; `None` when
/// nothing matches. `query` must already be lowercase.
fn chat_hit(chat_id: Option<u64>, file_ix: usize, title: SharedString, messages: &[ChatMessage], query: &str) -> Option<SidebarMsgHit> {
    if query.is_empty() {
        return None;
    }
    let mut count = 0;
    let mut newest = None;
    for (ix, m) in messages.iter().enumerate() {
        if crate::chat_search::msg_matches(m, query) {
            count += 1;
            newest = Some(ix);
        }
    }
    let msg_ix = newest?;
    Some(SidebarMsgHit {
        chat_id,
        file_ix,
        msg_ix,
        title,
        snippet: crate::chat_search::match_snippet(&messages[msg_ix], query, crate::chat_search::find_opts::FindOpts::default()).into(),
        count,
        at: messages[msg_ix].at,
    })
}

impl SidebarMsgHit {
    /// The dialog-shaped hit `open_hit` consumes — the sidebar row carries
    /// the same coordinates; context/stamps are render-only there.
    pub(crate) fn as_search_hit(&self) -> crate::global_search::SearchHit {
        crate::global_search::SearchHit {
            chat_id: self.chat_id,
            file_ix: self.file_ix,
            msg_ix: self.msg_ix,
            title: self.title.clone(),
            snippet: self.snippet.clone(),
            context: None,
            provider: String::new(),
            model: String::new(),
            at: self.at,
        }
    }
}

impl Workspace {
    /// A chat-list query edit: rescan live chats now, schedule the disk
    /// scan. `sidebar_search_gen` invalidates in-flight scans so a stale
    /// result can't overwrite a newer query's.
    pub(crate) fn schedule_sidebar_search(&mut self, cx: &mut Context<Self>) {
        self.sidebar_search_gen += 1;
        let stamp = self.sidebar_search_gen;
        let query = self.search.read(cx).value().trim().to_lowercase();
        if query.is_empty() {
            self.sidebar_hits.clear();
            self.sidebar_hits_extra = 0;
            return;
        }
        // Temporary chats are unsearchable — they never reach disk, and
        // pending transcripts aren't in memory — their files are scanned
        // in the disk half below (keyed to the live chat id).
        let mut hits: Vec<SidebarMsgHit> = self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.ephemeral && c.pending_load.is_none())
            .filter_map(|(ix, c)| chat_hit(Some(c.id), ix, c.title.clone(), &c.messages, &query))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.at));
        self.sidebar_hits_extra = hits.len().saturating_sub(MAX_ROWS);
        hits.truncate(MAX_ROWS);
        self.sidebar_hits = hits;

        // Disk half: chat files past the loaded set plus pending chats'
        // own files, parsed off the main thread after the debounce. A
        // pending chat's hits keep its live id so `open_hit` selects (and
        // hydrates) the real chat — but only when the file's `created_at`
        // still matches: another window may have rewritten the slots, and
        // a foreign file must not be attributed to this chat. Files that
        // drifted past the loaded set are matched back by `created_at` too
        // (a `None` id would load a duplicate on click).
        // `chat_files`/`read_stored` are the same helpers the Cmd-Shift-F
        // dialog scans with.
        let dir = self.project.chats_dir();
        let pending = PendingLookup {
            by_slot: self
                .chats
                .iter()
                .filter_map(|c| c.pending_load.map(|(slot, _)| (slot, (c.id, c.created_at))))
                .collect(),
            by_at: self.chats.iter().filter(|c| c.pending_load.is_some()).map(|c| (c.created_at, c.id)).collect(),
            live_ats: self.chats.iter().map(|c| c.created_at).collect(),
            live_len: self.chats.len(),
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let dir_hits = cx
                .background_executor()
                .spawn(async move {
                    let mut hits: Vec<SidebarMsgHit> = chat_files(&dir)
                        .into_iter()
                        .filter(|(ix, _)| *ix >= pending.live_len || pending.by_slot.contains_key(ix))
                        .filter(|(_, path)| path.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES))
                        .filter_map(|(ix, path)| read_stored(&path).map(|s| (ix, s)))
                        .filter_map(|(ix, s)| pending.live_id(ix, &s).and_then(|id| chat_hit(id, ix, s.title.into(), &s.messages, &query)))
                        .collect();
                    hits.sort_by_key(|h| std::cmp::Reverse(h.at));
                    hits
                })
                .await;
            this.update(cx, |this, cx| this.land_sidebar_disk_hits(stamp, dir_hits, cx)).ok();
        })
        .detach();
    }

    /// Merge a finished disk scan into `sidebar_hits` — dropped when a
    /// newer query already superseded it (`stamp` predates the current
    /// generation). `dir_hits` arrives sorted newest-first.
    fn land_sidebar_disk_hits(&mut self, stamp: u64, dir_hits: Vec<SidebarMsgHit>, cx: &mut Context<Self>) {
        if stamp != self.sidebar_search_gen {
            return;
        }
        // live_shown + live_extra + disk = every hit; the rows past
        // MAX_ROWS collapse into the "+N more" footer.
        let total = self.sidebar_hits.len() + self.sidebar_hits_extra + dir_hits.len();
        self.sidebar_hits.extend(dir_hits);
        self.sidebar_hits.sort_by_key(|h| std::cmp::Reverse(h.at));
        self.sidebar_hits.truncate(MAX_ROWS);
        self.sidebar_hits_extra = total - self.sidebar_hits.len();
        cx.notify();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "sidebar_search_tests.rs"]
mod sidebar_search_tests;
