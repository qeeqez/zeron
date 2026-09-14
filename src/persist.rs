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
            draft: chat.draft.clone(),
            created_at: chat.created_at,
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
            && stored.messages.iter().any(|m| matches!(&m.kind, MessageKind::Tool(t) if t.status == ToolStatus::Running));
        if foreign_turn
            && let Some(on_disk) =
                fs::read_to_string(&dst).ok().and_then(|s| serde_json::from_str::<StoredChat>(&s).ok())
            && on_disk.v == 1
            && on_disk.messages.len() >= stored.messages.len()
        {
            stored.messages = on_disk.messages;
        }
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
    /// Interface font size in px — also the rem base, so rem-sized UI text
    /// scales with it.
    pub font_size: u8,
    /// Interface font family; empty = system default (`.SystemUIFont`).
    pub font_family: String,
    /// Code (monospace) font family; empty = theme default (Menlo on macOS).
    pub code_font_family: String,
    /// Code font size in px — drives `Theme::mono_font_size`.
    pub code_font_size: u8,
    /// Chrome contrast percentage: 50 = muted, 100 = theme default, 200 = max.
    pub contrast: u16,
    /// Sidebar translucency: on = frosted glass over the blurred window
    /// background, off = opaque `theme.sidebar`.
    pub sidebar_frosted: bool,
    /// Last window bounds: [x, y, width, height] in pixels.
    pub window_bounds: Option<[f32; 4]>,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    /// Legacy field: per-project now (`projects/<id>/state.json`). Read for
    /// migration, never written back.
    #[serde(skip_serializing)]
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
            font_family: String::new(),
            code_font_family: String::new(),
            code_font_size: 13,
            contrast: 100,
            sidebar_frosted: true,
            window_bounds: None,
            sidebar_width: 255.0,
            sidebar_collapsed: false,
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
