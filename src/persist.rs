use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{Chat, ChatMessage, MessageKind, ToolStatus};

#[derive(Serialize, Deserialize)]
pub(crate) struct StoredChat {
    v: u32,
    title: String,
    messages: Vec<ChatMessage>,
    /// Missing in early v1 files.
    #[serde(default)]
    pub(crate) pinned: bool,
    /// Sidebar folder — missing in files written before folders existed;
    /// empty means "Unfiled".
    #[serde(default)]
    pub(crate) folder: String,
    #[serde(default)]
    pub(crate) archived: bool,
    #[serde(default)]
    pub(crate) draft: String,
    /// The title was auto-generated — missing in files written before
    /// chat titles existed; false lets an old chat still earn one.
    #[serde(default)]
    title_generated: bool,
    /// Missing in early v1 files — fall back to now().
    #[serde(default = "std::time::SystemTime::now")]
    created_at: std::time::SystemTime,
    /// Per-thread provider/model/access/workdir — missing in files written
    /// before thread defaults existed; empty means "follow the selection".
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    access: String,
    /// Reasoning effort override — missing/empty = the model's default.
    #[serde(default)]
    effort: String,
    #[serde(default)]
    workdir: String,
    #[serde(default)]
    worktree: bool,
    /// Backend thread the chat continues (resumed sessions); missing in
    /// files written before resume existed.
    #[serde(default)]
    thread_id: String,
    /// Per-turn workdir checkpoints — missing in files written before
    /// checkpoints existed.
    #[serde(default)]
    checkpoints: Vec<crate::checkpoints::TurnCheckpoint>,
    /// "What went wrong" notes on thumbs-down ratings — missing in files
    /// written before message feedback existed.
    #[serde(default)]
    pub(crate) feedback: Vec<crate::feedback::FeedbackNote>,
    /// Composer prompt history (Up/Down recall) — missing in files written
    /// before history existed.
    #[serde(default)]
    prompt_history: Vec<String>,
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
    for (ix, chat) in chats.iter().enumerate() {
        let mut stored = StoredChat {
            v: 1,
            title: chat.title.to_string(),
            messages: (*chat.messages).clone(),
            pinned: chat.pinned,
            archived: chat.archived,
            folder: chat.folder.clone(),
            draft: chat.draft.clone(),
            title_generated: chat.title_generated,
            created_at: chat.created_at,
            provider: chat.provider.clone(),
            model: chat.model.clone(),
            access: chat.access.map_or_else(String::new, |a| a.name().to_string()),
            workdir: chat.workdir.clone(),
            effort: chat.effort.clone().unwrap_or_default(),
            worktree: chat.worktree,
            thread_id: chat.thread_id.clone(),
            checkpoints: chat.checkpoints.clone(),
            feedback: chat.feedback.clone(),
            prompt_history: chat.prompt_history.clone(),
        };
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
    // Remove files beyond the live set — deleted chats must not resurrect.
    if let Ok(entries) = fs::read_dir(dir) {
        for path in entries.flatten().map(|e| e.path()) {
            let stale = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|ix| ix >= chats.len())
                && path.extension().is_some_and(|e| e == "json");
            if stale {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Load chats from `dir`; returns empty vec on any error. Files are read in
/// numeric-name order — the same order `save_chats` wrote — so the persisted
/// `active_chat` index still points at the same conversation. Each chat gets
/// a fresh id from `next_id` so reply tasks can target chats stably.
/// `recover_interrupted` marks tools saved mid-`Running` as failed — pass it
/// only on a cold start, when no live window can own those turns.
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
        .filter_map(|(_, path)| {
            let mut stored: StoredChat = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            if stored.v != 1 {
                // Unknown format — keep the file as .bak so it isn't lost.
                let _ = fs::rename(&path, path.with_extension("json.bak"));
                return None;
            }
            // A chat saved mid-turn leaves ToolStatus::Running behind. Only a
            // cold start may mark it failed — when another window owns a live
            // turn, rewriting its status here would persist a false failure.
            for m in &mut stored.messages {
                if recover_interrupted
                    && let MessageKind::Tool(t) = &mut m.kind
                    && t.status == ToolStatus::Running
                {
                    t.status = ToolStatus::Failed;
                }
            }
            let mut chat = Chat::new(*next_id, stored.title);
            *next_id += 1;
            chat.messages = std::rc::Rc::new(stored.messages);
            chat.pinned = stored.pinned;
            chat.archived = stored.archived;
            chat.folder = stored.folder;
            chat.draft = stored.draft;
            chat.title_generated = stored.title_generated;
            chat.created_at = stored.created_at;
            chat.provider = stored.provider;
            chat.model = stored.model;
            chat.access = if stored.access.is_empty() {
                None
            } else {
                Some(crate::backend::AccessMode::from_name(&stored.access))
            };
            chat.effort = if stored.effort.is_empty() { None } else { Some(stored.effort) };
            chat.workdir = stored.workdir;
            chat.worktree = stored.worktree;
            chat.checkpoints = stored.checkpoints;
            chat.thread_id = stored.thread_id;
            chat.feedback = stored.feedback;
            chat.prompt_history = stored.prompt_history;
            Some(chat)
        })
        .collect()
}

/// On-disk wrapper for the saved-prompts store so a future format bump can
/// reject unknown versions.
#[derive(Serialize, Deserialize)]
struct StoredPrompts {
    v: u32,
    prompts: Vec<crate::prompts::SavedPrompt>,
}

/// Write the saved prompts to `dir/prompts.json` (atomic tmp+rename). An
/// empty store removes the file so a cleared list stays cleared.
pub fn save_prompts(dir: &std::path::Path, store: &crate::prompts::PromptStore) {
    let path = dir.join("prompts.json");
    if store.prompts.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    let stored = StoredPrompts { v: 1, prompts: store.prompts.clone() };
    let Ok(json) = serde_json::to_string_pretty(&stored) else { return };
    // Skip the write when nothing changed — prompt mutations are rare but
    // the file stays byte-identical across unrelated saves.
    if fs::read_to_string(&path).is_ok_and(|old| old == json) {
        return;
    }
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join("prompts.json.tmp");
    let _ = fs::write(&tmp, json);
    let _ = fs::rename(&tmp, &path);
}

/// Read `dir/prompts.json`; an empty store on any error or unknown version.
pub fn load_prompts(dir: &std::path::Path) -> crate::prompts::PromptStore {
    fs::read_to_string(dir.join("prompts.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<StoredPrompts>(&s).ok())
        .filter(|s| s.v == 1)
        .map_or_else(crate::prompts::PromptStore::default, |s| crate::prompts::PromptStore { prompts: s.prompts })
}

pub use crate::persist_settings::{DefaultModel, Settings, load_settings, save_settings};

pub use crate::persist_model_cache::{load_model_cache, save_model_cache};
