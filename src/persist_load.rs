//! Re-reading transcripts `load_chats` deliberately skipped — the lazy
//! half of persistence. `chat_files`/`read_stored` are shared with the
//! global-search disk scan; `find_stored` locates the file a pending chat
//! came from even after another window rewrote slot numbers; `hydrate_chat`
//! attaches the real messages on first open.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::model::{Chat, ChatMessage, MessageKind, ToolStatus};
use crate::persist::StoredChat;

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
pub(crate) fn read_stored(path: &Path) -> Option<StoredChat> {
    let stored: StoredChat = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (stored.v == 1).then_some(stored)
}

/// A `messages` slot that skips the transcript on parse — `read_stored`
/// pays for the full payload, but `save_chats`'s every-save pending check
/// only needs metadata. `PartialEq` is trivially true, so a
/// `StoredChat<SkipMessages>` compares metadata only.
#[derive(PartialEq)]
pub(crate) struct SkipMessages;

impl<'de> serde::Deserialize<'de> for SkipMessages {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(d).map(|_| Self)
    }
}

/// The metadata half of `read_stored` — skips the transcript so a slot
/// probe costs a file read plus a skip-parse, not a full DOM.
pub(crate) fn read_meta(path: &Path) -> Option<StoredChat<SkipMessages>> {
    let stored: StoredChat<SkipMessages> = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (stored.v == 1).then_some(stored)
}

/// The fields `find_stored` needs to identify a chat's file — a `Chat`
/// holds `Rc`s so it can't cross to a background thread, but this probe
/// can (the pending-bookmark scan runs off the main thread).
pub(crate) struct ChatFileProbe {
    /// `pending_load`'s slot hint — the file's last known position.
    pub slot: usize,
    pub created_at: SystemTime,
    pub title: String,
    pub thread_id: String,
}

impl ChatFileProbe {
    pub(crate) fn of(chat: &Chat) -> Option<Self> {
        chat.pending_load.map(|(slot, _)| Self {
            slot,
            created_at: chat.created_at,
            title: chat.title.to_string(),
            thread_id: chat.thread_id.clone(),
        })
    }
}

/// The file `probe`'s transcript lives in. The slot names where it was
/// loaded from, but another window may have rewritten slot numbers
/// since — verify `created_at` still matches before trusting the hint,
/// then fall back to a full scan (the title+thread_id pair is a second
/// identity in case the timestamp can't match). Files written before
/// `created_at` existed never reach here — `load_chats` loads them
/// eagerly because this probe couldn't re-identify them.
pub(crate) fn find_stored(dir: &Path, probe: &ChatFileProbe) -> Option<StoredChat> {
    if let Some(stored) = read_stored(&dir.join(format!("{}.json", probe.slot)))
        && stored.created_at == Some(probe.created_at)
    {
        return Some(stored);
    }
    chat_files(dir).into_iter().find_map(|(_, path)| {
        let stored = read_stored(&path)?;
        (stored.created_at == Some(probe.created_at)
            || (stored.title == probe.title && !stored.thread_id.is_empty() && stored.thread_id == probe.thread_id))
            .then_some(stored)
    })
}

/// `find_stored` for every probe — each resolves its own file, so probes
/// fan out across a few scoped threads: a UI-thread caller like
/// `search_docs` shouldn't serialize N transcript parses. Results come
/// back in probe order.
pub(crate) fn find_stored_all(dir: &Path, probes: &[ChatFileProbe]) -> Vec<Option<StoredChat>> {
    if probes.len() <= 2 {
        return probes.iter().map(|p| find_stored(dir, p)).collect();
    }
    let chunk = probes.len().div_ceil(8);
    std::thread::scope(|s| {
        let handles: Vec<_> = probes
            .chunks(chunk)
            .map(|ch| s.spawn(|| ch.iter().map(|p| find_stored(dir, p)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("find_stored probe panicked")).collect()
    })
}

/// Attach the persisted transcript to a pending chat; `false` when the
/// file is gone (the chat stays empty — a later save rewrites it cleanly).
/// Replays `load_chats`' interrupted-turn recovery, and only on a cold
/// start — a live turn in another window keeps its `Running` status.
pub(crate) fn hydrate_chat(chat: &mut Chat, dir: &Path) -> bool {
    let Some((_, recover)) = chat.pending_load else { return true };
    // Live messages on a still-pending chat mean a mutation landed while
    // the file was unreadable — memory is authoritative now, and loading
    // the file back would silently drop them.
    if !chat.messages.is_empty() {
        chat.pending_load = None;
        chat.pending_has_plan = false;
        return true;
    }
    let Some(probe) = ChatFileProbe::of(chat) else { return true };
    let Some(mut stored) = find_stored(dir, &probe) else {
        // Nothing readable — keep `pending_load` so `save_chats` keeps
        // re-reading instead of overwriting the file with the empty
        // placeholder. The chat just looks empty until disk recovers.
        return false;
    };
    if recover {
        mark_interrupted(&mut stored.messages);
    }
    chat.messages = std::rc::Rc::new(stored.messages);
    chat.pending_load = None;
    chat.pending_has_plan = false;
    true
}

/// `load_chats`'s cold-start recovery, shared by its eager legacy-file
/// path and `hydrate_chat`: tools saved mid-`Running` get marked failed.
/// Call it only when no live window can own those turns.
pub(crate) fn mark_interrupted(messages: &mut [ChatMessage]) {
    for m in messages {
        if let MessageKind::Tool(t) = &mut m.kind
            && t.status == ToolStatus::Running
        {
            t.status = ToolStatus::Failed;
        }
    }
}

/// Test shorthand for the old eager-load behavior: hydrate every chat a
/// `load_chats` returned so assertions can read `.messages` directly.
#[cfg(test)]
pub(crate) fn hydrate_all(chats: &mut [Chat], dir: &Path) {
    for chat in chats {
        hydrate_chat(chat, dir);
    }
}

#[cfg(test)]
#[path = "persist_load_tests.rs"]
mod persist_load_tests;

#[cfg(test)]
#[path = "persist_load_guard_tests.rs"]
mod persist_load_guard_tests;
