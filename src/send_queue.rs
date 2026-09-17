//! Per-workspace send queue: messages committed while a reply runs.
//!
//! The queue lives on `Workspace` — not a shared thread_local — so two
//! windows whose chats reuse the same ids never see each other's queued
//! prompts, and a closed window drops its queue with the workspace.
//!
//! Queues persist to `queue.json` next to the chat files, keyed by each
//! chat's `created_at` — the only per-chat value stable across restarts
//! (chat ids are reassigned on load, file indices shift on delete). A
//! second window adopts a persisted queue only for chats it doesn't
//! already own, so live per-workspace scoping is preserved.

use std::collections::{HashMap, HashSet, VecDeque};

use gpui_kit::{Context, SharedString};
use serde::{Deserialize, Serialize};

use crate::model::Chat;
use crate::workspace::Workspace;

/// A message committed while a reply was running — sends when the turn ends.
/// `attachments` is a snapshot taken at submit time: enqueue clears the live
/// composer list, so later chips can't leak into (or be stripped from) an
/// already-queued prompt.
#[derive(Clone, Serialize, Deserialize)]
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

/// A queued message parked in the composer for editing. The item is out of
/// the queue until commit re-inserts it at `index`, so a turn ending
/// mid-edit can't drain the stale text. `saved_*` is what the composer held
/// before the edit — restored on commit so the draft survives.
pub struct QueueEdit {
    pub chat_id: u64,
    pub item: Queued,
    pub index: usize,
    pub saved_text: String,
    pub saved_attachments: Vec<SharedString>,
}

/// Composer queues keyed by chat id, scoped to one workspace window.
#[derive(Default)]
pub struct SendQueue {
    by_chat: HashMap<u64, VecDeque<Queued>>,
    /// Chat ids with an in-flight drain task — dedupes drain spawns.
    draining: HashSet<u64>,
    /// The queued message currently open in the composer, if any.
    editing: Option<QueueEdit>,
    next_id: u64,
    /// Persisted queues are adopted once, lazily, on first render.
    hydrated: bool,
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

    /// Count of a chat's queued messages — the sidebar badge's `+N`. The
    /// parked edit stays out: it's parked in the composer, not queued.
    pub fn len(&self, chat_id: u64) -> usize {
        self.by_chat.get(&chat_id).map_or(0, VecDeque::len)
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
        if self.editing.as_ref().is_some_and(|e| e.item.id == queued_id) {
            self.editing = None;
        }
        if let Some(d) = self.by_chat.get_mut(&chat_id) {
            d.retain(|item| item.id != queued_id);
        }
    }

    /// Move a queued message `delta` positions (negative = earlier). False
    /// when the id isn't queued on this chat or the move is a no-op.
    pub fn move_by(&mut self, chat_id: u64, queued_id: u64, delta: isize) -> bool {
        let Some(d) = self.by_chat.get_mut(&chat_id) else { return false };
        let Some(ix) = d.iter().position(|item| item.id == queued_id) else { return false };
        let to = ix.saturating_add_signed(delta).min(d.len() - 1);
        if to == ix {
            return false;
        }
        let item = d.remove(ix).expect("position checked");
        d.insert(to, item);
        true
    }

    /// "Send now": jump a queued message to the front of its chat's queue.
    /// No backend can inject into a running turn (`AgentBackend` exposes
    /// only `send`, and each turn spawns a fresh process), so steering
    /// falls back to sending next — immediately, when the chat is idle.
    pub fn move_to_front(&mut self, chat_id: u64, queued_id: u64) -> bool {
        self.move_by(chat_id, queued_id, isize::MIN)
    }

    /// True while a queued message on `chat_id` is open in the composer.
    pub fn editing_for(&self, chat_id: u64) -> bool {
        self.editing.as_ref().is_some_and(|e| e.chat_id == chat_id)
    }

    /// Park `queued_id` for editing: the item leaves the queue and the
    /// composer's current contents are stashed on the edit. Returns the
    /// item so the caller can load it into the composer.
    pub fn begin_edit(&mut self, chat_id: u64, queued_id: u64, saved_text: String, saved_attachments: Vec<SharedString>) -> Option<Queued> {
        let d = self.by_chat.get_mut(&chat_id)?;
        let index = d.iter().position(|item| item.id == queued_id)?;
        let item = d.remove(index).expect("position checked");
        if d.is_empty() {
            self.by_chat.remove(&chat_id);
        }
        let edit = QueueEdit {
            chat_id,
            item: item.clone(),
            index,
            saved_text,
            saved_attachments,
        };
        self.editing = Some(edit);
        Some(item)
    }

    /// Commit the parked edit: re-insert the item at its original position
    /// with `text`/`attachments`. Empty text cancels — the original message
    /// goes back untouched (deletion is the row's ✕, not a blank send).
    /// Returns the consumed edit so the caller can restore the stash.
    pub fn commit_edit(&mut self, text: String, attachments: Vec<SharedString>) -> Option<QueueEdit> {
        let mut edit = self.editing.take()?;
        let text = text.trim().to_string();
        if !text.is_empty() {
            edit.item.text = text;
            edit.item.attachments = attachments;
        }
        self.reinsert(edit.chat_id, edit.index, edit.item.clone());
        Some(edit)
    }

    /// Abandon the parked edit — the item returns with its original text.
    /// Used when the composer no longer holds the edit (chat switched).
    /// True when an edit was parked, so the caller can persist the restore.
    pub fn abandon_edit(&mut self) -> bool {
        if let Some(edit) = self.editing.take() {
            self.reinsert(edit.chat_id, edit.index, edit.item);
            return true;
        }
        false
    }

    /// Put an item back at `index`, clamped to the (possibly drained) queue.
    fn reinsert(&mut self, chat_id: u64, index: usize, item: Queued) {
        let d = self.by_chat.entry(chat_id).or_default();
        d.insert(index.min(d.len()), item);
    }

    /// Forget a chat's queue entirely (the chat was deleted).
    pub fn drop_chat(&mut self, chat_id: u64) {
        if self.editing.as_ref().is_some_and(|e| e.chat_id == chat_id) {
            self.editing = None;
        }
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

/// On-disk queue file: one entry per chat with pending messages, in chat
/// order. `key` is `chat_key` (created_at + title). Identical keys resolve
/// by position: each chat claims the first unclaimed entry with its key.
#[derive(Serialize, Deserialize)]
struct StoredQueues {
    v: u32,
    #[serde(default)]
    queues: Vec<StoredQueue>,
}

#[derive(Serialize, Deserialize)]
struct StoredQueue {
    key: String,
    items: Vec<Queued>,
}

/// A chat's persistence key: `created_at` nanos + title. `created_at` is
/// stable across restarts — unlike chat id (reassigned on load) or file
/// index (shifts on delete) — and the title disambiguates same-tick
/// creations so a deleted chat's twin can't inherit its queue. A rename
/// between the last queue mutation and quit drops the entry instead of
/// misrouting it — losing a queue beats sending it to the wrong chat.
fn chat_key(chat: &Chat) -> String {
    let nanos = chat
        .created_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_default();
    format!("{nanos}|{}", chat.title)
}

impl SendQueue {
    /// Write every live chat's queue to `dir/queue.json` (atomic tmp+rename).
    /// The parked edit counts as queued — a crash mid-edit must not lose it.
    /// Chats not in `chats` are dropped from the file; an all-empty write
    /// removes it so no stale queue outlives its chat.
    pub fn persist(&self, dir: &std::path::Path, chats: &[Chat]) {
        let mut queues: HashMap<u64, Vec<Queued>> = self.by_chat.iter().map(|(id, d)| (*id, d.iter().cloned().collect())).collect();
        if let Some(e) = &self.editing {
            let d = queues.entry(e.chat_id).or_default();
            d.insert(e.index.min(d.len()), e.item.clone());
        }
        let stored = StoredQueues {
            v: 1,
            queues: chats
                .iter()
                .filter(|chat| !chat.ephemeral)
                .filter_map(|chat| {
                    let items = queues.get(&chat.id)?;
                    (!items.is_empty()).then(|| StoredQueue { key: chat_key(chat), items: items.clone() })
                })
                .collect(),
        };
        let path = dir.join("queue.json");
        if stored.queues.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let Ok(json) = serde_json::to_string(&stored) else { return };
        // Skip the write when nothing changed — persist runs on every
        // queue mutation and most leave the file identical.
        if std::fs::read_to_string(&path).is_ok_and(|old| old == json) {
            return;
        }
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join("queue.json.tmp");
        let _ = std::fs::write(&tmp, json);
        let _ = std::fs::rename(&tmp, &path);
    }

    /// Adopt persisted queues for `chats` — once per workspace, on first
    /// render. A chat this window already queues keeps its live entries
    /// (another window may own them); stored ids are reassigned fresh.
    pub fn hydrate(&mut self, dir: &std::path::Path, chats: &[Chat]) {
        if self.hydrated {
            return;
        }
        self.hydrated = true;
        let Some(stored) = std::fs::read_to_string(dir.join("queue.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<StoredQueues>(&s).ok())
            .filter(|s| s.v == 1)
        else {
            return;
        };
        let mut claimed = vec![false; stored.queues.len()];
        for chat in chats {
            if self.by_chat.contains_key(&chat.id) {
                continue;
            }
            let key = chat_key(chat);
            // First unclaimed entry with this key — same-tick created_at
            // collisions resolve positionally, in chat order.
            let Some(ix) = (0..stored.queues.len()).find(|&i| !claimed[i] && stored.queues[i].key == key) else {
                continue;
            };
            claimed[ix] = true;
            let d = self.by_chat.entry(chat.id).or_default();
            for item in &stored.queues[ix].items {
                let id = self.next_id;
                self.next_id += 1;
                d.push_back(Queued { id, ..item.clone() });
            }
        }
    }
}

impl Workspace {
    /// Write the send queue to disk — called after every mutation so the
    /// file always mirrors what the composer shows.
    pub(crate) fn persist_queue(&self) {
        self.send_queue.persist(&self.project.chats_dir(), &self.chats);
    }

    /// Drop a queued message (the row's ✕).
    pub fn remove_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        let chat_id = self.chats[self.active].id;
        self.send_queue.remove(chat_id, id);
        self.persist_queue();
        cx.notify();
    }

    /// Move a queued message one step earlier/later in send order.
    pub fn move_queued(&mut self, id: u64, delta: isize, cx: &mut Context<Self>) {
        let chat_id = self.chats[self.active].id;
        if self.send_queue.move_by(chat_id, id, delta) {
            self.persist_queue();
            cx.notify();
        }
    }

    /// "Send now" on a queued row: no backend can steer a running turn, so
    /// the message jumps to the front — sent next, or immediately when the
    /// chat is idle. See `SendQueue::move_to_front`.
    pub fn send_queued_now(&mut self, id: u64, cx: &mut Context<Self>) {
        let chat_id = self.chats[self.active].id;
        if self.send_queue.move_to_front(chat_id, id) {
            self.persist_queue();
            self.spawn_queue_drain(chat_id, cx);
            cx.notify();
        }
    }
}
