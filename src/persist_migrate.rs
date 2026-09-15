//! Settings migration: fold pre-instances flat fields into the
//! `providers`/`selected_*` model. Split from `persist.rs` to stay under
//! the SLOC cap.

use crate::persist::Settings;
use crate::providers::{ProviderInstance, ProviderKind};

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Empty here — `load_settings` migrates legacy fields or fills
            // the built-in set; serde's field defaults must not pre-seed it.
            providers: Vec::new(),
            selected_provider: String::new(),
            selected_model: String::new(),
            mode: "Agent".into(),
            access: "auto".into(),
            default_model: crate::persist::DefaultModel::default(),
            default_permissions: String::new(),
            default_workspace: String::new(),
            notify_on_done: true,
            notify_sound: true,
            word_wrap: true,
            diff_mode: "unified".into(),
            preferred_editor: String::new(),
            legacy_backend: String::new(),
            legacy_model: String::new(),
            legacy_http_url: String::new(),
            legacy_http_key_env: String::new(),
            legacy_acp_command: String::new(),
            legacy_disabled_providers: Vec::new(),
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
            terminal_open: false,
            active_chat: 0,
            theme: "system".into(),
            mcp_servers: Vec::new(),
            snapshot_retention_days: None,
            snapshot_cap_mb: None,
            voice_enabled: false,
            voice_language: String::new(),
            voice_on_device: false,
            instructions: String::new(),
            update_last_check: None,
            update_latest: String::new(),
            update_skip: String::new(),
        }
    }
}

/// The built-in provider set — one enabled instance per kind, codex-cli
/// selected. Used when there's no file and nothing to migrate.
pub(crate) fn default_providers() -> Vec<ProviderInstance> {
    ProviderKind::ALL
        .into_iter()
        .map(|kind| ProviderInstance::new(kind, kind.info().label.to_string()))
        .collect()
}

impl Settings {
    /// The legacy backend selector, folding in the pre-`backend` bool.
    fn legacy_backend_name(&self) -> &str {
        match self.use_codex_cli {
            Some(true) => "codex-cli",
            Some(false) => "sim",
            None => self.legacy_backend.as_str(),
        }
    }

    /// Fold pre-instances fields into `providers`/`selected_*`. Runs when a
    /// file has no `providers` array but carries any legacy selector: one
    /// instance per known kind, enabled unless it was in
    /// `disabled_providers`, with the old http/acp config landing on its
    /// kind's instance. The old `backend` picks the selected instance; the
    /// old `model` carries over unless it was the synthetic "default".
    pub(crate) fn migrate_legacy(&mut self) {
        if !self.providers.is_empty() {
            return;
        }
        let legacy = !self.legacy_backend.is_empty()
            || self.use_codex_cli.is_some()
            || !self.legacy_model.is_empty()
            || !self.legacy_http_url.is_empty()
            || !self.legacy_disabled_providers.is_empty();
        if !legacy {
            return;
        }
        let selected = self.legacy_backend_name().to_string();
        self.providers = ProviderKind::ALL
            .into_iter()
            .map(|kind| {
                let mut p = ProviderInstance::new(kind, kind.info().label.to_string());
                p.enabled = !self.legacy_disabled_providers.contains(&p.id);
                match kind {
                    ProviderKind::Http => {
                        p.command = self.legacy_http_url.clone();
                        p.key_env = self.legacy_http_key_env.clone();
                    },
                    ProviderKind::Acp if !self.legacy_acp_command.is_empty() => {
                        p.command = self.legacy_acp_command.clone();
                    },
                    _ => {},
                }
                p
            })
            .collect();
        self.selected_provider = if self.providers.iter().any(|p| p.id == selected && p.enabled) {
            selected
        } else {
            self.providers.iter().find(|p| p.enabled).map_or_else(String::new, |p| p.id.clone())
        };
        self.selected_model = if self.legacy_model == "default" { String::new() } else { self.legacy_model.clone() };
        self.legacy_backend = String::new();
        self.legacy_model = String::new();
        self.legacy_http_url = String::new();
        self.legacy_http_key_env = String::new();
        self.legacy_acp_command = String::new();
        self.legacy_disabled_providers = Vec::new();
        self.use_codex_cli = None;
    }
}
