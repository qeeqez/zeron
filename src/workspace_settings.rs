//! Persisting `Workspace` state to settings.json — split from
//! `workspace.rs` to stay under the SLOC cap.

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn save(&mut self) {
        // Retention: drop oldest non-pinned chats beyond the cap. Storage
        // order is oldest-first, so eviction hits the oldest first.
        const MAX_CHATS: usize = 50;
        if self.chats.len() > MAX_CHATS {
            let dropped = evict_overflow(&mut self.chats, MAX_CHATS, self.project.root(), self.active);
            self.active = self.active.saturating_sub(dropped).min(self.chats.len().saturating_sub(1));
        }
        crate::persist::save_chats(&self.project.chats_dir(), &self.chats);
        self.project.save_state(&crate::project::ProjectState { active_chat: self.active });
    }

    pub(crate) fn save_settings(&mut self) {
        self.save_settings_inner(false);
    }

    /// `force_theme`: an explicit theme pick (`set_theme`) always writes its
    /// value, even when it equals `theme_persisted` — another window may have
    /// overwritten the file since, and value equality can't tell "unchanged"
    /// from "deliberately re-chosen".
    fn save_settings_inner(&mut self, force_theme: bool) {
        // Preserve fields this window doesn't own: window bounds are saved
        // at close, and the theme may have been changed by another window
        // since this one loaded its snapshot.
        let prev = crate::persist::load_settings();
        let theme = if force_theme || self.theme != self.theme_persisted {
            // This window changed the theme — persist its choice.
            self.theme.clone()
        } else {
            // Another window may have changed it — keep the file's value
            // and adopt it so later saves don't resurrect the stale one.
            self.theme = prev.theme.clone();
            prev.theme.clone()
        };
        self.theme_persisted = theme.clone();
        crate::persist::save_settings(&crate::persist::Settings {
            providers: self.providers.clone(),
            selected_provider: self.selected_provider.clone(),
            selected_model: self.model.to_string(),
            mode: self.mode.to_string(),
            // While untrusted the workspace access is clamped to Supervised
            // — the file keeps the user's configured mode so trusting the
            // folder restores it instead of persisting the clamp.
            access: if self.trusted { self.access.name().into() } else { prev.access },
            default_model: self.default_model.clone(),
            default_permissions: self.default_permissions.map_or_else(String::new, |a| a.name().to_string()),
            default_workspace: self.default_workspace.name().into(),
            word_wrap: self.word_wrap,
            diff_mode: self.diff_mode.name().into(),
            preferred_editor: self.preferred_editor.name().into(),
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            code_font_family: self.code_font_family.clone(),
            code_font_size: self.code_font_size,
            contrast: self.contrast,
            sidebar_frosted: self.sidebar_frosted,
            notify_on_done: self.notify_on_done,
            notify_sound: self.notify_sound,
            window_bounds: prev.window_bounds,
            // MCP servers live on the settings panel, not this window —
            // keep the file's list so an unrelated save can't drop them.
            mcp_servers: prev.mcp_servers,
            snapshot_retention_days: Some(self.snapshots.retention_days),
            snapshot_cap_mb: Some(self.snapshots.cap_mb),
            voice_enabled: self.voice.enabled,
            voice_language: self.voice.language.clone(),
            voice_on_device: self.voice.on_device,
            instructions: self.instructions.clone(),
            // Update-check bookkeeping is written by `crate::update`, not
            // this window — keep the file's values so an unrelated save
            // can't drop a pending or skipped release.
            update_last_check: prev.update_last_check,
            update_latest: prev.update_latest,
            update_skip: prev.update_skip,
            // Trusted folders are written by `crate::trust`, not this
            // window — keep the file's list so an unrelated save can't
            // drop a folder the user trusted in another window.
            trusted_folders: prev.trusted_folders,
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
            terminal_open: self.terminal.open,
            plan_panel_open: self.plan_panel.open,
            theme,
            ..Default::default()
        });
    }

    /// Set the Changes panel's diff layout (unified | split) and persist it.
    /// A no-op pick still notifies so the toggle's pressed state re-renders.
    pub fn set_diff_mode(&mut self, mode: crate::changes_diff::DiffMode, cx: &mut Context<Self>) {
        self.diff_mode = mode;
        self.save_settings();
        cx.notify();
    }

    /// Set the appearance mode ("system" | "light" | "dark"), persist it and
    /// re-apply. The single write path for theme changes — theme cards and
    /// the palette's theme commands both land here.
    pub fn set_theme(&mut self, theme: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.theme = theme.to_string();
        self.save_settings_inner(true);
        self.apply_theme(window, cx);
    }

    /// Set the global custom instructions and persist them —
    /// `Settings.instructions`. The next turn's `TurnContext` picks them up.
    pub fn set_instructions(&mut self, instructions: String, cx: &mut Context<Self>) {
        self.instructions = instructions;
        self.save_settings();
        cx.notify();
    }

    /// The Custom Instructions section's Save button: copy the field's
    /// text into `instructions` and persist.
    pub(crate) fn save_instructions(&mut self, cx: &mut Context<Self>) {
        let text = self.instructions_input.read(cx).value().to_string();
        self.set_instructions(text, cx);
    }
}

/// Drop the oldest non-pinned chats beyond `max`, removing dropped
/// worktree checkouts under `root`. Returns how many dropped chats sat
/// before `active` so the caller can shift the index.
fn evict_overflow(chats: &mut Vec<crate::model::Chat>, max: usize, root: &std::path::Path, active: usize) -> usize {
    let mut drop_left = chats.len().saturating_sub(max);
    let mut dropped_before = 0usize;
    let mut kept = Vec::with_capacity(chats.len().min(max));
    for (ix, c) in std::mem::take(chats).into_iter().enumerate() {
        if drop_left > 0 && !c.pinned {
            crate::worktree::remove_for(root, &c);
            drop_left -= 1;
            dropped_before += usize::from(ix < active);
        } else {
            kept.push(c);
        }
    }
    *chats = kept;
    dropped_before
}
