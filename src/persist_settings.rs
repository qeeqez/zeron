//! Settings persistence — split from `persist.rs` for the SLOC cap.
//! Re-exported from `persist` so callers keep `crate::persist::*` paths.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::persist::dirs_home;

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
    /// macOS notification when a reply lands off-screen — a background
    /// chat's turn finishing, or any turn while the window is unfocused.
    /// Off = replies only ever get the in-app toast.
    pub notify_background: bool,
    pub word_wrap: bool,
    /// Changes-panel diff layout — a `DiffMode::name` ("unified" | "split").
    pub diff_mode: String,
    /// Changes-panel diffs hide whitespace-only changes — `git diff
    /// --ignore-all-space`; the header's space toggle.
    pub diff_ignore_ws: bool,
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
    /// scales with it. Half-px steps in [10, 20].
    pub font_size: f32,
    /// Interface font family; empty = system default (`.SystemUIFont`).
    pub font_family: String,
    /// Code (monospace) font family; empty = theme default (Menlo on macOS).
    pub code_font_family: String,
    /// Code font size in px — drives `Theme::mono_font_size`.
    pub code_font_size: f32,
    /// Chrome contrast percentage: 50 = muted, 100 = theme default, 200 = max.
    pub contrast: u16,
    /// Sidebar translucency: on = frosted glass over the blurred window
    /// background, off = opaque `theme.sidebar`.
    pub sidebar_frosted: bool,
    /// Last window bounds: [x, y, width, height] in pixels.
    pub window_bounds: Option<[f32; 4]>,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    /// Bottom terminal panel open/closed — restored on launch.
    pub terminal_open: bool,
    /// Plan panel open/closed — restored on launch.
    pub plan_panel_open: bool,
    /// Scheduled panel open/closed — restored on launch.
    pub scheduled_panel_open: bool,
    /// Bookmarks panel open/closed — restored on launch.
    pub bookmarks_panel_open: bool,
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
    /// Global default spend cap in USD — a chat whose accumulated cost
    /// crosses it raises the budget banner (`None` = no cap). A chat's own
    /// `budget_alert_usd` overrides this; see `crate::chat_ops::budget`.
    #[serde(default)]
    pub budget_alert_usd: Option<f64>,
    /// Voice dictation master switch — the composer's mic button and
    /// Cmd-Shift-D only record when this is on.
    pub voice_enabled: bool,
    /// BCP-47 locale for the speech recognizer; empty = system default.
    pub voice_language: String,
    /// Prefer on-device speech recognition (more private, fewer languages).
    pub voice_on_device: bool,
    /// Global custom instructions — prepended to every turn's system
    /// context ahead of the project's own instructions file (see
    /// `crate::instructions`).
    pub instructions: String,
    /// Last update-check time — gates the daily automatic check (see
    /// `crate::update`). `None` = never checked.
    pub update_last_check: Option<std::time::SystemTime>,
    /// Newest release tag seen by the last check; empty = up to date. Kept
    /// so the About row can show a pending update before the next fetch.
    pub update_latest: String,
    /// Release tag the user dismissed — it won't notify again, though a
    /// newer tag still does.
    pub update_skip: String,
    /// Canonicalized project roots the user has trusted — untrusted folders
    /// open in restricted mode (see `crate::trust`).
    #[serde(default = "Vec::new")]
    pub trusted_folders: Vec<String>,
    /// First-run onboarding card dismissed — the empty state stops pushing
    /// provider setup once the user skips it (a configured provider hides
    /// it regardless).
    #[serde(default)]
    pub onboarding_dismissed: bool,
    /// System-wide summon hotkey: `global_hotkey_enabled` installs the
    /// `NSEvent` global monitor, `global_hotkey` is the chord in gpui
    /// keystroke syntax ("cmd-shift-space"; empty = the default). See
    /// `crate::app_setup::global_hotkey`.
    #[serde(default)]
    pub global_hotkey_enabled: bool,
    #[serde(default)]
    pub global_hotkey: String,
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
