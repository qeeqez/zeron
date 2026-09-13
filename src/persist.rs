use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{Chat, ChatMessage};

#[derive(Serialize, Deserialize)]
struct StoredChat {
    v: u32,
    title: String,
    messages: Vec<ChatMessage>,
    pinned: bool,
    archived: bool,
    draft: String,
    /// Missing in early v1 files — fall back to now().
    #[serde(default = "std::time::SystemTime::now")]
    created_at: std::time::SystemTime,
}

pub(crate) fn chats_dir() -> PathBuf {
    dirs_home().join(".rixl/rixlcode/chats")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME").map_or_else(|_| PathBuf::from("/tmp"), PathBuf::from)
}

/// Save all chats to disk (atomic tmp+rename per file). Files for chats
/// that no longer exist are removed so deletions survive restarts.
pub fn save_chats(chats: &[Chat]) {
    let dir = chats_dir();
    let _ = fs::create_dir_all(&dir);
    for (ix, chat) in chats.iter().enumerate() {
        let stored = StoredChat {
            v: 1,
            title: chat.title.to_string(),
            messages: chat.messages.clone(),
            pinned: chat.pinned,
            archived: chat.archived,
            draft: chat.draft.clone(),
            created_at: chat.created_at,
        };
        let tmp = dir.join(format!("{ix}.json.tmp"));
        let dst = dir.join(format!("{ix}.json"));
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
    if let Ok(entries) = fs::read_dir(&dir) {
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

/// Load chats from disk; returns empty vec on any error. Files are read in
/// numeric-name order — the same order `save_chats` wrote — so the persisted
/// `active_chat` index still points at the same conversation. Each chat gets
/// a fresh id from `next_id` so reply tasks can target chats stably.
pub fn load_chats(next_id: &mut u64) -> Vec<Chat> {
    let dir = chats_dir();
    let Ok(entries) = fs::read_dir(&dir) else { return Vec::new() };
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
            let stored: StoredChat = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
            if stored.v != 1 {
                return None;
            }
            let mut chat = Chat::new(*next_id, stored.title);
            *next_id += 1;
            chat.messages = stored.messages;
            chat.pinned = stored.pinned;
            chat.archived = stored.archived;
            chat.draft = stored.draft;
            chat.created_at = stored.created_at;
            Some(chat)
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub model: String,
    pub mode: String,
    pub notify_on_done: bool,
    pub word_wrap: bool,
    pub use_codex_cli: bool,
    pub font_size: u8,
    /// Last window bounds: [x, y, width, height] in pixels.
    pub window_bounds: Option<[f32; 4]>,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    pub active_chat: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: "default".into(),
            mode: "Agent".into(),
            notify_on_done: true,
            word_wrap: true,
            use_codex_cli: true,
            font_size: 14,
            window_bounds: None,
            sidebar_width: 255.0,
            sidebar_collapsed: false,
            active_chat: 0,
        }
    }
}

fn settings_path() -> PathBuf {
    dirs_home().join(".rixl/rixlcode/settings.json")
}

pub fn save_settings(s: &Settings) {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = fs::write(&tmp, json);
        let _ = fs::rename(&tmp, &path);
    }
}

pub fn load_settings() -> Settings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
