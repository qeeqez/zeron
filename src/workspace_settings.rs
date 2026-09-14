//! Persisting `Workspace` state to settings.json — split from
//! `workspace.rs` to stay under the SLOC cap.

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn save_settings(&mut self) {
        // Preserve fields this window doesn't own: window bounds are saved
        // at close, and the theme may have been changed by another window
        // since this one loaded its snapshot.
        let prev = crate::persist::load_settings();
        let theme = if self.theme != self.theme_persisted {
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
            model: self.model.to_string(),
            mode: self.mode.to_string(),
            access: self.access.name().into(),
            word_wrap: self.word_wrap,
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            code_font_family: self.code_font_family.clone(),
            code_font_size: self.code_font_size,
            contrast: self.contrast,
            sidebar_frosted: self.sidebar_frosted,
            notify_on_done: self.notify_on_done,
            backend: self.backend.name().into(),
            http_url: self.http_url.clone(),
            http_key_env: self.http_key_env.clone(),
            use_codex_cli: None,
            window_bounds: prev.window_bounds,
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
            active_chat: 0,
            theme,
        });
    }

    /// Set the appearance mode ("system" | "light" | "dark"), persist it and
    /// re-apply. The single write path for theme changes — theme cards and
    /// the palette's theme commands both land here.
    pub fn set_theme(&mut self, theme: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.theme = theme.to_string();
        self.save_settings();
        self.apply_theme(window, cx);
    }
}
