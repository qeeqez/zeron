//! The single source of truth for workspace keybindings: `workspace_keys`
//! registers every bound row into the keymap and the shortcuts overlay
//! (`views::shortcuts`) renders the same table, so the cheat sheet can never
//! drift from what the keys actually do. `bind` is `None` for rows that
//! describe behavior handled outside the keymap (e.g. the composer's Enter).

use gpui_kit::*;

/// Overlay grouping — the section a row is listed under.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShortcutGroup {
    General,
    Composer,
    Navigation,
    Settings,
}

impl ShortcutGroup {
    pub const ALL: [ShortcutGroup; 4] =
        [ShortcutGroup::General, ShortcutGroup::Composer, ShortcutGroup::Navigation, ShortcutGroup::Settings];

    pub fn label(self) -> &'static str {
        match self {
            ShortcutGroup::General => "General",
            ShortcutGroup::Composer => "Composer",
            ShortcutGroup::Navigation => "Navigation",
            ShortcutGroup::Settings => "Settings",
        }
    }
}

/// One row of the cheat sheet: the keymap keystroke, the binding factory that
/// registers it (`None` for display-only rows), and what the user sees.
pub struct ShortcutSpec {
    /// Keymap syntax, e.g. "cmd-shift-n" — also what the row's chips show.
    pub keys: &'static str,
    /// Builds the `KeyBinding` for `workspace_keys`; `None` = not bound.
    pub bind: Option<fn(&'static str) -> KeyBinding>,
    pub description: &'static str,
    pub group: ShortcutGroup,
}

macro_rules! spec {
    ($keys:literal, $action:ident, $desc:literal, $group:ident) => {
        ShortcutSpec {
            keys: $keys,
            bind: Some({
                fn bind(keys: &'static str) -> KeyBinding {
                    KeyBinding::new(keys, crate::$action, Some("workspace"))
                }
                bind
            }),
            description: $desc,
            group: ShortcutGroup::$group,
        }
    };
    ($keys:literal, $desc:literal, $group:ident) => {
        ShortcutSpec {
            keys: $keys,
            bind: None,
            description: $desc,
            group: ShortcutGroup::$group,
        }
    };
}

/// Every shortcut the overlay lists. Bound rows must match the keymap exactly
/// — `workspace_keys` registers this table and nothing else.
pub static SHORTCUT_SPECS: &[ShortcutSpec] = &[
    spec!("escape", EscapeKey, "Dismiss overlay / stop reply / close panels", General),
    spec!("cmd-/", ShortcutsHelp, "Keyboard shortcuts", General),
    spec!("cmd-n", NewChat, "New chat", General),
    spec!("cmd-shift-n", NewWindow, "New window", General),
    spec!("cmd-q", QuitApp, "Quit Rixl Code", General),
    spec!("cmd-h", HideApp, "Hide Rixl Code", General),
    spec!("cmd-alt-h", HideOthers, "Hide other windows", General),
    spec!("cmd-m", MinimizeWindow, "Minimize window", General),
    spec!("ctrl-cmd-f", EnterFullscreen, "Toggle full screen", General),
    spec!("cmd-w", CloseWindow, "Close window", General),
    spec!("cmd-shift-backspace", DeleteChat, "Delete chat", General),
    spec!("cmd-b", ToggleSidebar, "Toggle sidebar", General),
    spec!("cmd-j", ToggleAgents, "Toggle agents panel", General),
    spec!("cmd-shift-j", ToggleChanges, "Toggle changes panel", General),
    spec!("cmd-shift-e", ToggleExplorer, "Toggle file explorer", General),
    spec!("enter", "Send message", Composer),
    spec!("shift-enter", "New line", Composer),
    spec!("cmd-up", RecallLast, "Recall last message", Composer),
    spec!("cmd-shift-up", RecallPrev, "Recall previous message", Composer),
    spec!("cmd-shift-down", RecallNext, "Recall next message", Composer),
    spec!("cmd-k", OpenPalette, "Command palette", Navigation),
    spec!("cmd-f", FindInChat, "Find in chat", Navigation),
    spec!("cmd-1", Chat1, "Switch to chat 1", Navigation),
    spec!("cmd-2", Chat2, "Switch to chat 2", Navigation),
    spec!("cmd-3", Chat3, "Switch to chat 3", Navigation),
    spec!("cmd-4", Chat4, "Switch to chat 4", Navigation),
    spec!("cmd-5", Chat5, "Switch to chat 5", Navigation),
    spec!("cmd-6", Chat6, "Switch to chat 6", Navigation),
    spec!("cmd-7", Chat7, "Switch to chat 7", Navigation),
    spec!("cmd-8", Chat8, "Switch to chat 8", Navigation),
    spec!("cmd-9", Chat9, "Switch to chat 9", Navigation),
    spec!("cmd-,", OpenSettings, "Open settings", Settings),
];
