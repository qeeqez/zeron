//! Per-type notification granularity — the settings surface matching the
//! real app's Notifications section: a "Turn completion notifications"
//! pick (Never / Only when unfocused / Always) plus an independent
//! "Permission notifications" switch for approval prompts.
//!
//! The toggles persist to `~/.rixl/rixlcode/notify.json` — a sidecar of
//! `settings.json` (`persist_settings.rs` belongs to another lane), with
//! the same atomic tmp+rename write `activity.json` uses.

use gpui_kit::*;

use crate::workspace::Workspace;

/// When a turn event reaches the OS notification center — the real app's
/// "Turn completion notifications" options. The in-app toast is a
/// separate surface gated by `Workspace::notify_on_done`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyTiming {
    /// Never post to the OS.
    Never,
    /// Post only while the chat isn't on screen — a background chat or an
    /// unfocused window (the `notify_background` flag).
    Unfocused,
    /// Post even while the finished chat is on screen.
    Always,
}

impl NotifyTiming {
    /// Segmented-row order: escalating reach.
    pub const ALL: [Self; 3] = [Self::Never, Self::Unfocused, Self::Always];

    /// The row's button label — the real app's option names.
    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::Unfocused => "Only when unfocused",
            Self::Always => "Always",
        }
    }

    /// The (`always`, `background`) flag pair an option selects — `always`
    /// subsumes unfocused delivery, so Always writes both on.
    fn flags(self) -> (bool, bool) {
        match self {
            Self::Never => (false, false),
            Self::Unfocused => (false, true),
            Self::Always => (true, true),
        }
    }

    /// The option a flag pair encodes — `always` dominates since it
    /// subsumes the unfocused case.
    pub fn current(always: bool, background: bool) -> Self {
        if always {
            Self::Always
        } else if background {
            Self::Unfocused
        } else {
            Self::Never
        }
    }
}

/// Notification toggles that don't live in `Settings` — `notify.json`'s
/// whole contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NotifyPrefs {
    /// "Always" delivery: post system notifications even while the chat
    /// is watched. Pairs with `Workspace::notify_background` — together
    /// they encode `NotifyTiming`.
    pub always: bool,
    /// Approval prompts post their own toast/system surfaces independent
    /// of the turn-completion toggles — the real app's "Enable permission
    /// notifications". Defaults on: approvals alerted before it existed.
    pub approvals: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self { always: false, approvals: true }
    }
}

/// `~/.rixl/rixlcode/notify.json` — next to `settings.json`.
fn path() -> std::path::PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode/notify.json")
}

impl NotifyPrefs {
    /// Read the sidecar; defaults on any error — a missing file is the
    /// common case (the toggles are new).
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Atomic tmp+rename write — the `activity.json` shape.
    pub fn persist(&self) {
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&tmp, json);
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Should a turn event reach the OS notification center? `watched` =
    /// the chat is on screen (active AND the window focused). "Always"
    /// posts regardless; "Only when unfocused" (`background`) needs the
    /// chat off screen; "Never" stays in-app.
    pub fn system(&self, background: bool, watched: bool) -> bool {
        self.always || (background && !watched)
    }
}

impl Workspace {
    /// OS delivery for a turn event on this workspace — folds the
    /// workspace's `notify_background` flag into the sidecar's `always`.
    pub(crate) fn notify_system(&self, watched: bool) -> bool {
        self.notify_prefs.system(self.notify_background, watched)
    }

    /// The segmented row's pick: write the (`always`, `notify_background`)
    /// pair and persist both stores — `always` lives in `notify.json`,
    /// `notify_background` in `settings.json`.
    pub fn set_notify_timing(&mut self, timing: NotifyTiming, cx: &mut Context<Self>) {
        let (always, background) = timing.flags();
        self.notify_prefs.always = always;
        self.notify_background = background;
        self.notify_prefs.persist();
        self.save_settings();
        cx.notify();
    }

    /// The "Permission notifications" switch: approval prompts get their
    /// own gate, independent of the turn-completion toggles.
    pub fn set_notify_approvals(&mut self, on: bool, cx: &mut Context<Self>) {
        self.notify_prefs.approvals = on;
        self.notify_prefs.persist();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{NotifyPrefs, NotifyTiming};

    #[test]
    fn timing_current_maps_flag_pairs() {
        assert_eq!(NotifyTiming::current(false, false), NotifyTiming::Never);
        assert_eq!(NotifyTiming::current(false, true), NotifyTiming::Unfocused);
        assert_eq!(NotifyTiming::current(true, false), NotifyTiming::Always, "always subsumes background");
        assert_eq!(NotifyTiming::current(true, true), NotifyTiming::Always);
    }

    #[test]
    fn timing_flags_roundtrip() {
        for t in NotifyTiming::ALL {
            let (always, background) = t.flags();
            assert_eq!(NotifyTiming::current(always, background), t);
        }
    }

    #[test]
    fn prefs_default_keeps_approvals_on() {
        let prefs = NotifyPrefs::default();
        assert!(!prefs.always);
        assert!(prefs.approvals, "approvals alerted before the toggle existed — default stays on");
    }

    #[test]
    fn system_delivery_matrix() {
        let never = NotifyPrefs { always: false, approvals: true };
        let always = NotifyPrefs { always: true, approvals: true };
        assert!(!never.system(false, false) && !never.system(false, true), "Never stays in-app");
        assert!(never.system(true, false) && !never.system(true, true), "Unfocused needs an unwatched chat");
        assert!(always.system(false, false) && always.system(false, true), "Always posts regardless");
        assert!(always.system(true, true));
    }

    #[test]
    fn persist_roundtrips() {
        let dir = std::env::temp_dir().join(format!("rixlcode-prefs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: nextest runs each test in its own process, so no other
        // thread can observe HOME mid-write.
        unsafe { std::env::set_var("HOME", &dir) };
        let prefs = NotifyPrefs { always: true, approvals: false };
        prefs.persist();
        assert_eq!(NotifyPrefs::load(), prefs, "a written sidecar should load back identically");
    }

    #[test]
    fn load_defaults_on_missing_or_corrupt() {
        let dir = std::env::temp_dir().join(format!("rixlcode-prefs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: nextest runs each test in its own process, so no other
        // thread can observe HOME mid-write.
        unsafe { std::env::set_var("HOME", &dir) };
        assert_eq!(NotifyPrefs::load(), NotifyPrefs::default(), "no file means defaults");
        let file = dir.join(".rixl/rixlcode/notify.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(NotifyPrefs::load(), NotifyPrefs::default(), "a corrupt file means defaults");
    }
}
