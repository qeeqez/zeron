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

/// Load chats from disk; returns empty vec on any error.
pub fn load_chats() -> Vec<Chat> {
    let dir = chats_dir();
    let Ok(entries) = fs::read_dir(&dir) else { return Vec::new() };
    let mut chats: Vec<Chat> = entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let stored: StoredChat = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
            if stored.v != 1 {
                return None;
            }
            let mut chat = Chat::new(stored.title);
            chat.messages = stored.messages;
            chat.pinned = stored.pinned;
            chat.archived = stored.archived;
            chat.draft = stored.draft;
            chat.created_at = stored.created_at;
            Some(chat)
        })
        .collect();
    chats.sort_by_key(|c| c.created_at);
    chats
}

/// Keep at most `MAX_CHATS` files; delete oldest beyond that.
pub fn enforce_retention(chats: &[Chat]) {
    const MAX_CHATS: usize = 50;
    if chats.len() <= MAX_CHATS {
        return;
    }
    let dir = chats_dir();
    for ix in MAX_CHATS..chats.len() {
        let _ = fs::remove_file(dir.join(format!("{ix}.json")));
    }
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
