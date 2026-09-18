use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{Chat, ChatMessage, MessageKind, ToolStatus};

/// The `messages` field is generic so `load_chats` can parse metadata-only
/// (`IgnoredAny`) without paying for transcripts nobody opened yet, and
/// `recover_interrupted` can take just the messages to mutate.
#[derive(Serialize, Deserialize, PartialEq)]
pub(crate) struct StoredChat<M = Vec<ChatMessage>> {
    pub(crate) v: u32,
    pub(crate) title: String,
    pub(crate) messages: M,
    /// Missing in early v1 files.
    #[serde(default)]
    pub(crate) pinned: bool,
    /// Sidebar folder — missing in files written before folders existed;
    /// empty means "Unfiled".
    #[serde(default)]
    pub(crate) folder: String,
    #[serde(default)]
    pub(crate) archived: bool,
    /// Unsent composer text — skipped when empty so cleared drafts don't
    /// leave a `"draft": ""` key behind.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) draft: String,
    /// The title was auto-generated — missing in files written before
    /// chat titles existed; false lets an old chat still earn one.
    #[serde(default)]
    pub(crate) title_generated: bool,
    /// The user named this chat — missing in files written before the
    /// flag existed; false is safe, an old renamed chat still fails the
    /// placeholder check.
    #[serde(default)]
    pub(crate) title_custom: bool,
    /// Missing in early v1 files — `None` marks them so `load_chats` can
    /// load them eagerly: a lazy-load probe can't re-identify a file whose
    /// timestamp was never persisted (each parse would stamp a different
    /// `now()`). Writes always store `Some`.
    #[serde(default)]
    pub(crate) created_at: Option<std::time::SystemTime>,
    /// Manual sidebar position — missing in files written before drag
    /// reorder existed; `0` falls back to `created_at` ordering.
    #[serde(default)]
    pub(crate) order: i64,
    /// Per-thread provider/model/access/workdir — missing in files written
    /// before thread defaults existed; empty means "follow the selection".
    #[serde(default)]
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) access: String,
    /// Reasoning effort override — missing/empty = the model's default.
    #[serde(default)]
    pub(crate) effort: String,
    #[serde(default)]
    pub(crate) workdir: String,
    #[serde(default)]
    pub(crate) worktree: bool,
    /// The ref a worktree chat's Changes panel diffs against — missing in
    /// files written before the diff-base picker existed.
    #[serde(default)]
    pub(crate) diff_base: Option<String>,
    /// Backend thread the chat continues — bound on the first turn's
    /// `ThreadBound` event or when the chat resumed a past session.
    #[serde(default)]
    pub(crate) thread_id: String,
    /// Per-turn workdir checkpoints — missing in files written before
    /// checkpoints existed.
    #[serde(default)]
    pub(crate) checkpoints: Vec<crate::checkpoints::TurnCheckpoint>,
    /// "What went wrong" notes on thumbs-down ratings — missing in files
    /// written before message feedback existed.
    #[serde(default)]
    pub(crate) feedback: Vec<crate::feedback::FeedbackNote>,
    /// Composer prompt history (Up/Down recall) — missing in files written
    /// before history existed.
    #[serde(default)]
    pub(crate) prompt_history: Vec<String>,
    /// Per-chat custom instructions — missing in files written before
    /// per-chat instructions existed; empty means no override.
    #[serde(default)]
    pub(crate) instructions: String,
    /// Color tag for visual grouping — missing in files written before
    /// color tags existed; unknown names load as untagged.
    #[serde(default)]
    pub(crate) color: String,
    /// Per-chat spend cap in USD — missing in files written before budget
    /// alerts existed; `None` rides the global default.
    #[serde(default)]
    pub(crate) budget_alert_usd: Option<f64>,
}

impl<M> StoredChat<M> {
    /// The live `Chat` this file becomes plus its messages payload —
    /// shared by `load_chats` and the global-search single-file load so
    /// neither drops fields the other restores. The messages come back
    /// separately so a lazy load can drop the `IgnoredAny` placeholder
    /// without a dummy `Rc`.
    pub(crate) fn into_chat(self, id: u64) -> (Chat, M) {
        let mut chat = Chat::new(id, self.title);
        chat.messages = std::rc::Rc::new(Vec::new());
        chat.pinned = self.pinned;
        chat.archived = self.archived;
        chat.folder = self.folder;
        chat.draft = self.draft;
        chat.title_generated = self.title_generated;
        chat.title_custom = self.title_custom;
        // `None` marks a pre-`created_at` file — stamp it now; the value
        // only identifies this run's in-memory chat.
        chat.created_at = self.created_at.unwrap_or_else(std::time::SystemTime::now);
        chat.order = self.order;
        chat.provider = self.provider;
        chat.model = self.model;
        chat.access = if self.access.is_empty() {
            None
        } else {
            Some(crate::backend::AccessMode::from_name(&self.access))
        };
        chat.effort = if self.effort.is_empty() { None } else { Some(self.effort) };
        chat.workdir = self.workdir;
        chat.worktree = self.worktree;
        chat.diff_base = self.diff_base;
        chat.checkpoints = self.checkpoints;
        chat.thread_id = self.thread_id;
        chat.feedback = self.feedback;
        chat.prompt_history = self.prompt_history;
        chat.color = crate::model::ChatColor::from_name(&self.color);
        chat.instructions = if self.instructions.is_empty() { None } else { Some(self.instructions) };
        chat.budget_alert_usd = self.budget_alert_usd;
        (chat, self.messages)
    }
}

/// Chats dir for the current project — kept for `RevealChats` in root.rs.
pub(crate) fn chats_dir() -> PathBuf {
    crate::project::Project::current().chats_dir()
}

pub(crate) fn dirs_home() -> PathBuf {
    std::env::var("HOME").map_or_else(|_| PathBuf::from("/tmp"), PathBuf::from)
}

/// Save all chats to `dir` (atomic tmp+rename per file). Files for chats
/// that no longer exist are removed so deletions survive restarts.
pub fn save_chats(dir: &std::path::Path, chats: &[Chat]) {
    let _ = fs::create_dir_all(dir);
    // Unhydrated chats carry an empty in-memory transcript — writing it
    // would wipe history. Re-read each pending chat's file first (keyed by
    // chat id, before the slot rewriting below) so metadata saves keep the
    // real messages. `find_stored` verifies the hinted slot still holds
    // this chat — a stale hint after another window rewrote slots must not
    // graft a neighbor's transcript onto these metadata fields.
    let mut deferred = std::collections::HashMap::new();
    for (ix, chat) in chats.iter().enumerate() {
        // A pending chat already carrying live messages diverged from its
        // file while the file was unreadable — memory is authoritative
        // (see `hydrate_chat`), so there is nothing to re-read.
        if chat.pending_load.is_none() || !chat.messages.is_empty() {
            continue;
        }
        let Some(probe) = persist_load::ChatFileProbe::of(chat) else { continue };
        // The chat's own slot still holds its file with identical metadata
        // — the file is already correct, so skip the transcript re-read.
        // `read_meta` ignores `messages`, keeping this check cheap on the
        // every-save path. The `probe.slot == ix` gate matters: a file
        // anywhere else is the drift case `find_stored` handles — another
        // position's write (or the stale sweep) would take it.
        if probe.slot == ix
            && persist_load::read_meta(&dir.join(format!("{}.json", probe.slot)))
                .is_some_and(|meta| meta == stored_fields(chat, persist_load::SkipMessages))
        {
            continue;
        }
        if let Some(messages) = persist_load::find_stored(dir, &probe).map(|stored| stored.messages) {
            deferred.insert(chat.id, messages);
        }
    }
    for (ix, chat) in chats.iter().enumerate() {
        // Temporary chats never reach disk — the slot stays empty and the
        // stale sweep below removes any file that ever lands there.
        if chat.ephemeral {
            continue;
        }
        // An unhydrated chat must not persist its empty placeholder — use
        // the transcript re-read above. A missing read means the file
        // vanished or changed hands mid-save, so leave the disk copy as
        // is — unless the chat has since grown live messages; those must
        // persist rather than silently drop (memory is authoritative —
        // `hydrate_chat` clears `pending_load` on the same rule).
        let messages = match chat.pending_load {
            Some(_) => match deferred.remove(&chat.id) {
                Some(m) => m,
                None if !chat.messages.is_empty() => (*chat.messages).clone(),
                None => continue,
            },
            None => (*chat.messages).clone(),
        };
        let mut stored = stored_fields(chat, messages);
        let tmp = dir.join(format!("{ix}.json.tmp"));
        let dst = dir.join(format!("{ix}.json"));
        // A Running tool in a chat this workspace isn't running belongs to
        // another window's live turn — our copy is a stale snapshot with no
        // reply task behind it. Re-read the file before overwriting: when
        // the on-disk transcript has at least as many messages, the owning
        // window's write is newer, so keep its messages and persist only
        // our metadata (a longer local list means this window continued
        // the conversation itself — ours wins).
        let foreign_turn = !chat.running
            && stored
                .messages
                .iter()
                .any(|m| matches!(&m.kind, MessageKind::Tool(t) if t.status == ToolStatus::Running));
        if foreign_turn
            && let Some(on_disk) = fs::read_to_string(&dst).ok().and_then(|s| serde_json::from_str::<StoredChat>(&s).ok())
            && on_disk.v == 1
            // `dst` is just a slot — after another window rewrote the map
            // it can hold a different chat entirely, so graft only the
            // same chat's transcript (the `created_at` identity
            // `find_stored` verifies too).
            && on_disk.created_at == stored.created_at
            && on_disk.messages.len() >= stored.messages.len()
        {
            stored.messages = on_disk.messages;
        }
        // Drop notes whose anchor message is gone (truncated by an edit or
        // /clear, or swapped out by the foreign-turn merge above).
        stored.feedback.retain(|n| stored.messages.iter().any(|m| m.at == n.at));
        if let Ok(json) = serde_json::to_string_pretty(&stored) {
            // Skip the write when nothing changed — save() runs on every
            // keystroke-adjacent action and most chats are untouched.
            if fs::read_to_string(&dst).is_ok_and(|old| old == json) {
                continue;
            }
            let _ = fs::write(&tmp, json);
            let _ = fs::rename(&tmp, &dst);
        }
    }
    // Slots past the live set — or held by an ephemeral chat — are stale:
    // deleted chats and temporary chats must not resurrect.
    if let Ok(entries) = fs::read_dir(dir) {
        for path in entries.flatten().map(|e| e.path()) {
            let stale = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|ix| chats.get(ix).is_none_or(|c| c.ephemeral))
                && path.extension().is_some_and(|e| e == "json");
            if stale {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// The `Chat` → `StoredChat` field mapping — generic over the messages
/// payload so the pending-chat fast path can compare metadata without a
/// transcript (`StoredChat<SkipMessages>`).
fn stored_fields<M>(chat: &Chat, messages: M) -> StoredChat<M> {
    StoredChat {
        v: 1,
        title: chat.title.to_string(),
        messages,
        pinned: chat.pinned,
        archived: chat.archived,
        folder: chat.folder.clone(),
        draft: chat.draft.clone(),
        title_generated: chat.title_generated,
        title_custom: chat.title_custom,
        created_at: Some(chat.created_at),
        order: chat.order,
        provider: chat.provider.clone(),
        model: chat.model.clone(),
        access: chat.access.map_or_else(String::new, |a| a.name().to_string()),
        workdir: chat.workdir.clone(),
        effort: chat.effort.clone().unwrap_or_default(),
        worktree: chat.worktree,
        diff_base: chat.diff_base.clone(),
        thread_id: chat.thread_id.clone(),
        checkpoints: chat.checkpoints.clone(),
        feedback: chat.feedback.clone(),
        prompt_history: chat.prompt_history.clone(),
        color: chat.color.map_or_else(String::new, |c| c.name().to_string()),
        instructions: chat.instructions.clone().unwrap_or_default(),
        budget_alert_usd: chat.budget_alert_usd,
    }
}

/// Load chats from `dir`; returns empty vec on any error. Files are read in
/// numeric-name order — the same order `save_chats` wrote — so the persisted
/// `active_chat` index still points at the same conversation. Each chat gets
/// a fresh id from `next_id` so reply tasks can target chats stably.
///
/// Transcripts stay on disk: messages parse as `IgnoredAny` so startup only
/// pays for metadata. `Chat::pending_load` records the slot the transcript
/// lives in plus whether `recover_interrupted` applies — `hydrate_chat`
/// replays it on first open. Pass `recover_interrupted` only on a cold
/// start, when no live window can own those turns. Files written before
/// `created_at` existed load eagerly — a `pending_load` probe can't
/// re-identify a file whose timestamp was never persisted.
pub fn load_chats(dir: &std::path::Path, next_id: &mut u64, recover_interrupted: bool) -> Vec<Chat> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
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
        .into_iter()
        .filter_map(|(ix, path)| {
            let raw = fs::read_to_string(&path).ok()?;
            let stored: StoredChat<serde::de::IgnoredAny> = serde_json::from_str(&raw).ok()?;
            if stored.v != 1 {
                // Unknown format — keep the file as .bak so it isn't lost.
                let _ = fs::rename(&path, path.with_extension("json.bak"));
                return None;
            }
            if stored.created_at.is_none() {
                // Written before `created_at` existed — a `find_stored`
                // probe could never re-identify this file, so the lazy
                // path can't serve it. Load the transcript eagerly, the
                // way `load_chats` always did; the next save stamps
                // `created_at` and the file rejoins the lazy path.
                let full: StoredChat = serde_json::from_str(&raw).ok()?;
                let (mut chat, mut messages) = full.into_chat(*next_id);
                if recover_interrupted {
                    persist_load::mark_interrupted(&mut messages);
                }
                chat.messages = std::rc::Rc::new(messages);
                *next_id += 1;
                return Some(chat);
            }
            let (mut chat, _) = stored.into_chat(*next_id);
            // `hydrate_chat` re-reads this file on first open and replays
            // the interrupted-tool recovery then — marking a live turn in
            // another window failed here would persist a false failure.
            chat.pending_load = Some((ix, recover_interrupted));
            *next_id += 1;
            Some(chat)
        })
        .collect()
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[path = "persist_automations.rs"]
mod persist_automations;
#[path = "persist_load.rs"]
pub(crate) mod persist_load;
#[path = "persist_prompts.rs"]
mod persist_prompts;
#[path = "persist_templates.rs"]
mod persist_templates;

#[cfg(test)]
pub(crate) use persist_load::hydrate_all;
pub(crate) use persist_load::{ChatFileProbe, chat_files, find_stored, find_stored_all, hydrate_chat, read_stored};
pub use persist_prompts::{load_prompts, save_prompts};

pub use crate::persist_settings::{DefaultModel, Settings, load_settings, save_settings};

pub use crate::persist_model_cache::{load_model_cache, save_model_cache};

pub use persist_automations::{load_automations, save_automations};
pub use persist_templates::{load_templates, save_templates};
