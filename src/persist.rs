use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{Chat, ChatMessage, MessageKind, ToolStatus};

#[derive(Serialize, Deserialize)]
struct StoredChat {
    v: u32,
    title: String,
    messages: Vec<ChatMessage>,
    /// Missing in early v1 files.
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
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
            messages: (*chat.messages).clone(),
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
            let mut stored: StoredChat = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            if stored.v != 1 {
                // Unknown format — keep the file as .bak so it isn't lost.
                let _ = fs::rename(&path, path.with_extension("json.bak"));
                return None;
            }
            // A chat saved mid-turn leaves ToolStatus::Running behind; the
            // owning turn is gone after restart, so mark it failed rather
            // than resurrect a spinner that can never resolve.
            for m in &mut stored.messages {
                if let MessageKind::Tool(t) = &mut m.kind
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
            chat.draft = stored.draft;
            chat.created_at = stored.created_at;
            Some(chat)
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: String,
    pub mode: String,
    /// Agent-mode filesystem access: "read-only" | "workspace-write" |
    /// "full-access" — see `crate::backend::AccessMode`.
    pub access: String,
    pub notify_on_done: bool,
    pub word_wrap: bool,
    /// Backend selector: "codex-cli" | "sim" | "http". Migrated from the
    /// old `use_codex_cli` bool — see `use_codex_cli` below.
    pub backend: String,
    /// HTTP transport endpoint (POST, NDJSON response stream).
    pub http_url: String,
    /// Env var holding the bearer token for `http_url` — the key itself
    /// is never written to this file.
    pub http_key_env: String,
    /// Legacy field: present only in pre-`backend` files. Read for
    /// migration, never written back.
    #[serde(skip_serializing)]
    pub use_codex_cli: Option<bool>,
    pub font_size: u8,
    /// Last window bounds: [x, y, width, height] in pixels.
    pub window_bounds: Option<[f32; 4]>,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    /// Settings-screen nav rail width in pixels.
    pub settings_nav_width: f32,
    pub active_chat: usize,
    /// Appearance: "system" | "light" | "dark".
    pub theme: String,
}

impl Settings {
    /// Effective backend name, folding in the legacy bool when the file
    /// predates the `backend` field.
    pub fn backend_name(&self) -> &str {
        match self.use_codex_cli {
            Some(true) => "codex-cli",
            Some(false) => "sim",
            None => self.backend.as_str(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: "default".into(),
            mode: "Agent".into(),
            access: "workspace-write".into(),
            notify_on_done: true,
            word_wrap: true,
            backend: "codex-cli".into(),
            http_url: String::new(),
            http_key_env: "RIXL_API_KEY".into(),
            use_codex_cli: None,
            font_size: 14,
            window_bounds: None,
            sidebar_width: 255.0,
            sidebar_collapsed: false,
            settings_nav_width: 220.0,
            active_chat: 0,
            theme: "system".into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_chat_defaults_missing_fields() {
        // Early v1 files lack pinned/archived/draft/created_at — they must
        // parse with defaults instead of dropping the chat.
        let json = r#"{"v":1,"title":"t","messages":[]}"#;
        let s: StoredChat = serde_json::from_str(json).unwrap();
        assert!(!s.pinned && !s.archived && s.draft.is_empty());
    }

    #[test]
    fn settings_defaults_missing_fields() {
        // A file with only `model` must not reset the rest.
        let s: Settings = serde_json::from_str(r#"{"model":"gpt-5"}"#).unwrap();
        assert_eq!(s.model, "gpt-5");
        assert_eq!(s.font_size, 14);
        assert!(s.notify_on_done);
    }

    #[test]
    fn settings_roundtrip() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, s.model);
        assert_eq!(back.font_size, s.font_size);
    }
}
