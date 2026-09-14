//! Per-workspace send queue: messages committed while a reply runs.
//!
//! The queue lives on `Workspace` — not a shared thread_local — so two
//! windows whose chats reuse the same ids never see each other's queued
//! prompts, and a closed window drops its queue with the workspace.

use std::collections::{HashMap, HashSet, VecDeque};

use gpui_kit::SharedString;

/// A message committed while a reply was running — sends when the turn ends.
/// `attachments` is a snapshot taken at submit time: enqueue clears the live
/// composer list, so later chips can't leak into (or be stripped from) an
/// already-queued prompt.
#[derive(Clone)]
pub struct Queued {
    pub id: u64,
    pub text: String,
    pub attachments: Vec<SharedString>,
}

impl Queued {
    /// A not-yet-queued message — `SendQueue::enqueue` assigns the real id.
    pub fn new(text: String, attachments: Vec<SharedString>) -> Self {
        Self { id: 0, text, attachments }
    }
}

/// Composer queues keyed by chat id, scoped to one workspace window.
#[derive(Default)]
pub struct SendQueue {
    by_chat: HashMap<u64, VecDeque<Queued>>,
    /// Chat ids with an in-flight drain task — dedupes drain spawns.
    draining: HashSet<u64>,
    next_id: u64,
}

impl SendQueue {
    /// Queue `item` behind the running turn on `chat_id`, assigning its real
    /// id; `live` prunes entries for chats that no longer exist.
    pub fn enqueue(&mut self, chat_id: u64, item: Queued, live: impl Fn(u64) -> bool) {
        self.by_chat.retain(|id, _| live(*id));
        let id = self.next_id;
        self.next_id += 1;
        self.by_chat.entry(chat_id).or_default().push_back(Queued { id, ..item });
    }

    /// Snapshot of a chat's queue for rendering.
    pub fn queued(&self, chat_id: u64) -> Vec<Queued> {
        self.by_chat.get(&chat_id).map(|d| d.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn pop(&mut self, chat_id: u64) -> Option<Queued> {
        let item = self.by_chat.get_mut(&chat_id)?.pop_front();
        if self.by_chat.get(&chat_id).is_some_and(VecDeque::is_empty) {
            self.by_chat.remove(&chat_id);
        }
        item
    }

    /// Drop one queued message (the ✕ on a queued row).
    pub fn remove(&mut self, chat_id: u64, queued_id: u64) {
        if let Some(d) = self.by_chat.get_mut(&chat_id) {
            d.retain(|item| item.id != queued_id);
        }
    }

    /// Forget a chat's queue entirely (the chat was deleted).
    pub fn drop_chat(&mut self, chat_id: u64) {
        self.by_chat.remove(&chat_id);
    }

    /// Mark a drain task in flight for `chat_id`; false when one already runs.
    pub fn draining_begin(&mut self, chat_id: u64) -> bool {
        self.draining.insert(chat_id)
    }

    pub fn draining_end(&mut self, chat_id: u64) {
        self.draining.remove(&chat_id);
    }
}
