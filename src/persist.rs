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
    #[serde(default)]
    pub(crate) archived: bool,
    #[serde(default)]
    pub(crate) draft: String,
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
            provider: chat.provider.clone(),
            model: chat.model.clone(),
            access: chat.access.map_or_else(String::new, |a| a.name().to_string()),
            workdir: chat.workdir.clone(),
            effort: chat.effort.clone().unwrap_or_default(),
            worktree: chat.worktree,
            thread_id: chat.thread_id.clone(),
            checkpoints: chat.checkpoints.clone(),
            feedback: chat.feedback.clone(),
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
            chat.draft = stored.draft;
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
            Some(chat)
        })
        .collect()
}

/// The `default_model` setting — which provider instance + model new
/// threads start on. Either field may be empty: an empty provider follows
/// the current selection, an empty model resolves to the provider's first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultModel {
    pub provider_instance_id: String,
    pub model_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Configured provider instances — the picker's provider list. Empty in
    /// a fresh file means "migrate the legacy flat fields" (see
    /// `migrate_legacy`); `load_settings` fills it with the built-in set
    /// when there's nothing to migrate.
    #[serde(default = "Vec::new")]
    pub providers: Vec<crate::providers::ProviderInstance>,
    /// Selected instance id — an entry in `providers`.
    pub selected_provider: String,
    /// Selected model id within `selected_provider`'s catalog. Empty = the
    /// provider's first model; there is no synthetic "default" anymore.
    pub selected_model: String,
    pub mode: String,
    /// Agent-mode filesystem access — an `AccessMode::name` (legacy files
    /// may carry "read-only"/"workspace-write"; `from_name` maps them).
    pub access: String,
    /// Provider+model new threads start on; empty fields = the current
    /// selection. Existing threads keep their own stamped values.
    pub default_model: DefaultModel,
    /// Access mode new threads start on — an `AccessMode::name`; empty =
    /// the current workspace access.
    pub default_permissions: String,
    /// Where new threads run: "checkout" | "worktree" — see
    /// `crate::worktree::WorkspaceMode`.
    pub default_workspace: String,
    pub notify_on_done: bool,
    /// System bell on turn completion — a separate toggle from the
    /// toast/system notification, so the sound can play without a popup.
    pub notify_sound: bool,
    pub word_wrap: bool,
    /// Preferred editor for "Open in Editor" — a `PreferredEditor::name`
    /// ("vscode" | "cursor" | "zed" | "finder" | "ask"); empty = Ask.
    pub preferred_editor: String,
    /// Legacy field: the pre-instances backend selector ("codex-cli" |
    /// "claude-cli" | "sim" | "http" | "acp"). Read for migration, never
    /// written back.
    #[serde(rename = "backend", skip_serializing)]
    pub legacy_backend: String,
    /// Legacy field: the pre-instances model selector ("default" | a model
    /// id). Read for migration, never written back.
    #[serde(rename = "model", skip_serializing)]
    pub legacy_model: String,
    /// Legacy field: the http provider's endpoint. Read for migration,
    /// never written back — it lives on the instance's `command` now.
    #[serde(rename = "http_url", skip_serializing)]
    pub legacy_http_url: String,
    /// Legacy field: env var holding the http bearer token.
    #[serde(rename = "http_key_env", skip_serializing)]
    pub legacy_http_key_env: String,
    /// Legacy field: the acp provider's spawn command.
    #[serde(rename = "acp_command", skip_serializing)]
    pub legacy_acp_command: String,
    /// Legacy field: provider ids hidden from the picker — instance
    /// `enabled` flags now. Read for migration, never written back.
    #[serde(rename = "disabled_providers", skip_serializing)]
    pub legacy_disabled_providers: Vec<String>,
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
    /// Configured MCP servers — the settings section's list; enabled ones
    /// are mirrored into `~/.codex/config.toml` `[mcp_servers]` and passed
    /// to ACP `session/new`.
    #[serde(default = "Vec::new")]
    pub mcp_servers: Vec<crate::mcp::McpServer>,
    /// Snapshot retention: days before auto-prune (`None` = default 30,
    /// `Some(0)` = forever) and total size cap in MiB (`None`/`Some(0)` =
    /// no cap). See `crate::snapshots`.
    pub snapshot_retention_days: Option<u32>,
    pub snapshot_cap_mb: Option<u32>,
    /// Voice dictation master switch — the composer's mic button and
    /// Cmd-Shift-D only record when this is on.
    pub voice_enabled: bool,
    /// BCP-47 locale for the speech recognizer; empty = system default.
    pub voice_language: String,
    /// Prefer on-device speech recognition (more private, fewer languages).
    pub voice_on_device: bool,
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
    let mut s: Settings = fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    s.migrate_legacy();
    if s.providers.is_empty() {
        // Fresh profile — no file, nothing to migrate.
        s.providers = crate::persist_migrate::default_providers();
        s.selected_provider = crate::providers::ProviderKind::CodexCli.slug().to_string();
    }
    crate::mcp_config::import_codex_servers(&mut s);
    s
}

pub use crate::persist_model_cache::{load_model_cache, save_model_cache};
