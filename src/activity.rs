//! The activity feed: a bounded, per-project log of what finished or needs
//! attention — completed turns, approval prompts, failures — surfaced by the
//! titlebar bell's unread badge and the dropdown panel in `views::activity`.
//!
//! Entries persist to `<project>/activity.json` (atomic tmp+rename, same as
//! `send_queue`'s `queue.json`) so the feed survives restarts. Chat ids are
//! reassigned on every load, so entries link to their chat by `created_at` —
//! the same stable key `send_queue::chat_key` relies on.

use std::time::SystemTime;

use gpui_kit::*;

use crate::model::{Chat, MessageKind};
use crate::workspace::Workspace;

/// The feed keeps at most this many entries — the oldest drop off first.
pub const ACTIVITY_LIMIT: usize = 100;

/// What happened — drives the row's icon and the click behavior (approval
/// entries also scroll the pending card into view).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActivityKind {
    /// A backend turn completed.
    TurnFinished,
    /// The backend is blocked on the user's decision.
    Approval,
    /// The turn failed.
    Error,
    /// Housekeeping worth seeing — e.g. a deleted chat's dirty worktree
    /// left on disk.
    Note,
}

/// One feed row. `chat_title`/`body` are snapshots — a later rename or
/// deletion must not rewrite history.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ActivityEntry {
    pub kind: ActivityKind,
    /// The chat's title when the event was recorded.
    pub chat_title: SharedString,
    /// Preview line: reply excerpt, error detail, or the approval's command.
    pub body: String,
    /// Stable link back to the chat — `Chat::created_at` survives restarts
    /// while `Chat::id` is reassigned on every load.
    pub chat_created: SystemTime,
    pub at: SystemTime,
    /// Unread entries drive the bell's badge and the row's dot; opening
    /// the entry's chat clears them.
    pub unread: bool,
}

impl ActivityEntry {
    pub(crate) fn new(kind: ActivityKind, chat: &Chat, body: String) -> Self {
        Self {
            kind,
            chat_title: chat.title.clone(),
            body,
            chat_created: chat.created_at,
            at: SystemTime::now(),
            unread: true,
        }
    }
}

/// The activity log — oldest first, capped at `ACTIVITY_LIMIT`.
#[derive(Default)]
pub struct ActivityFeed {
    pub entries: Vec<ActivityEntry>,
}

/// On-disk wrapper so a future format bump can reject unknown versions.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredActivity {
    v: u32,
    entries: Vec<ActivityEntry>,
}

impl ActivityFeed {
    /// Append an entry, dropping the oldest beyond `ACTIVITY_LIMIT`.
    pub fn push(&mut self, entry: ActivityEntry) {
        self.entries.push(entry);
        let overflow = self.entries.len().saturating_sub(ACTIVITY_LIMIT);
        if overflow > 0 {
            self.entries.drain(..overflow);
        }
    }

    /// Entries newest-first — the panel's display order.
    pub fn recent(&self) -> impl Iterator<Item = (usize, &ActivityEntry)> {
        self.entries.iter().enumerate().rev()
    }

    /// Unread entries — the bell's badge count.
    pub fn unread_count(&self) -> usize {
        self.entries.iter().filter(|e| e.unread).count()
    }

    /// Mark every entry for `chat_created` read; returns whether anything
    /// changed so callers only persist on a real transition.
    pub fn mark_chat_read(&mut self, chat_created: SystemTime) -> bool {
        let mut changed = false;
        for e in &mut self.entries {
            if e.chat_created == chat_created {
                changed |= e.unread;
                e.unread = false;
            }
        }
        changed
    }

    /// Write the feed to `dir/activity.json` (atomic tmp+rename). An empty
    /// feed removes the file so a cleared feed stays cleared.
    pub fn persist(&self, dir: &std::path::Path) {
        let path = dir.join("activity.json");
        if self.entries.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let stored = StoredActivity { v: 1, entries: self.entries.clone() };
        let Ok(json) = serde_json::to_string(&stored) else { return };
        // Skip the write when nothing changed — persist runs on every feed
        // mutation and most leave the file identical.
        if std::fs::read_to_string(&path).is_ok_and(|old| old == json) {
            return;
        }
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join("activity.json.tmp");
        let _ = std::fs::write(&tmp, json);
        let _ = std::fs::rename(&tmp, &path);
    }

    /// Read `dir/activity.json`; an empty feed on any error or unknown
    /// version. Entries past the cap are dropped oldest-first.
    pub fn load(dir: &std::path::Path) -> Self {
        let Some(stored) = std::fs::read_to_string(dir.join("activity.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<StoredActivity>(&s).ok())
            .filter(|s| s.v == 1)
        else {
            return Self::default();
        };
        let mut feed = Self { entries: stored.entries };
        let overflow = feed.entries.len().saturating_sub(ACTIVITY_LIMIT);
        if overflow > 0 {
            feed.entries.drain(..overflow);
        }
        feed
    }
}

impl Workspace {
    /// Record a finished turn — `Error` when `failed_flag` is set, else
    /// `TurnFinished` with a reply preview. Called from `notify_done` so it
    /// fires regardless of the toast/sound toggles.
    pub(crate) fn record_turn_finished(&mut self, chat_id: u64) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        if chat.ephemeral {
            return;
        }
        let (kind, body) = if chat.failed_flag {
            (
                ActivityKind::Error,
                Self::error_detail(chat).map_or_else(|| "Reply failed".to_string(), |line| format!("Reply failed — {line}")),
            )
        } else {
            (ActivityKind::TurnFinished, Self::reply_preview(chat).unwrap_or_else(|| "Reply complete".to_string()))
        };
        self.push_activity(ActivityEntry::new(kind, chat, body));
    }

    /// Record a backend approval prompt — the card's kind + detail tell the
    /// user what needs a decision without opening the chat.
    pub(crate) fn record_approval(&mut self, chat_id: u64, kind: crate::backend::ApprovalKind, detail: &str) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        if chat.ephemeral {
            return;
        }
        let body = format!("{}: {detail}", kind.label());
        self.push_activity(ActivityEntry::new(ActivityKind::Approval, chat, body));
    }

    /// Push an entry and mirror the feed to disk — every mutation persists
    /// so the file always matches what the panel shows.
    pub(crate) fn push_activity(&mut self, entry: ActivityEntry) {
        self.activity.push(entry);
        self.persist_activity();
    }

    /// Record that a deleted chat's worktree stayed on disk — `remove`
    /// refused it (uncommitted work), so the feed keeps a pointer. Built
    /// before the chat drops: the entry snapshots its title and
    /// `created_at` link.
    pub(crate) fn worktree_kept_entry(chat: &Chat, reason: &str) -> ActivityEntry {
        ActivityEntry::new(ActivityKind::Note, chat, format!("Worktree kept ({reason}): {}", chat.workdir))
    }

    /// The clear-all variant of `worktree_kept_entry`: every chat is gone,
    /// so the note is synthetic — `chat_created` matches nothing and the
    /// row's click is inert.
    pub(crate) fn note_kept_worktrees(&mut self, kept: &[std::path::PathBuf]) {
        if kept.is_empty() {
            return;
        }
        let names = kept
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect::<Vec<_>>()
            .join(", ");
        self.push_activity(ActivityEntry {
            kind: ActivityKind::Note,
            chat_title: "Worktrees".into(),
            body: format!("{} worktree(s) left on disk (uncommitted changes): {names}", kept.len()),
            chat_created: SystemTime::now(),
            at: SystemTime::now(),
            unread: true,
        });
    }

    /// Write the feed into the project store.
    pub(crate) fn persist_activity(&self) {
        self.activity.persist(self.project.dir());
    }

    /// The bell toggles the panel. Viewing the list doesn't clear the
    /// badge — an entry's dot clears when its chat is opened.
    pub fn toggle_activity_panel(&mut self, cx: &mut Context<Self>) {
        self.activity_open = !self.activity_open;
        cx.notify();
    }

    /// Empty the feed and persist the removal.
    pub fn clear_activity(&mut self, cx: &mut Context<Self>) {
        self.activity.entries.clear();
        self.persist_activity();
        cx.notify();
    }

    /// Open the chat an entry points at. Approval entries also scroll the
    /// pending card into view; a deleted chat leaves the entry inert.
    pub fn open_activity_entry(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.activity_open = false;
        let Some(entry) = self.activity.entries.get(ix) else {
            cx.notify();
            return;
        };
        let kind = entry.kind;
        let chat_created = entry.chat_created;
        // Overlay state is cleared too — a click while settings is open must
        // reveal the chat, not leave the overlay covering it.
        self.settings_open = false;
        // The row led somewhere, so the chat's unread dots are done even
        // when the chat itself is gone.
        self.mark_chat_activity_read(chat_created);
        if let Some(ix) = self.chats.iter().position(|c| c.created_at == chat_created) {
            self.select_chat(ix, window, cx);
            if kind == ActivityKind::Approval {
                self.scroll_to_pending_approval(cx);
            }
        }
        cx.notify();
    }

    /// Remove one row — the per-row × next to the header's Clear-all.
    pub fn dismiss_activity_entry(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.activity.entries.len() {
            self.activity.entries.remove(ix);
            self.persist_activity();
        }
        cx.notify();
    }

    /// Clear the unread flag on every entry for a chat — called wherever
    /// the chat is opened (sidebar select, activity row, notice click).
    pub(crate) fn mark_chat_activity_read(&mut self, chat_created: SystemTime) {
        if self.activity.mark_chat_read(chat_created) {
            self.persist_activity();
        }
    }

    /// Scroll the active chat to its pending approval card — the last
    /// unanswered one, or the last card when all are answered (the entry
    /// outlived the prompt).
    fn scroll_to_pending_approval(&mut self, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let is_card = |m: &crate::model::ChatMessage| matches!(m.kind, MessageKind::Approval(_));
        let ix = chat
            .messages
            .iter()
            .rposition(|m| matches!(&m.kind, MessageKind::Approval(a) if a.decision.is_none()))
            .or_else(|| chat.messages.iter().rposition(is_card));
        if let Some(ix) = ix {
            let pos = self.filtered_pos(ix, cx);
            self.scroller.update(cx, |s, cx| {
                s.scroll_to_item(pos, cx);
            });
        }
    }
}

// Declared here, not in `main.rs` — that file is at the SLOC cap.
#[cfg(test)]
#[path = "activity_ui_tests.rs"]
mod activity_ui_tests;
