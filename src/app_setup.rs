//! App-level setup: the workspace keymap and global action handlers —
//! split from `main.rs` for the SLOC cap.

use gpui_kit::*;

use crate::{
    AboutApp, BringAllToFront, CheckForUpdates, FindInChat, HideApp, HideOthers, MsgNavDown, MsgNavTop, MsgNavUp, NewWindow, OpenProject,
    QuitApp, SearchAllChats, TogglePlan, ToggleTerminal, lifecycle, menus, shortcuts, update,
};

/// Workspace-context key bindings. Menu items pick their key equivalents up
/// from these, so a shortcut added here shows in the menu bar for free. The
/// list comes from `shortcuts::SHORTCUT_SPECS` — the same table the Cmd-/
/// overlay renders, so the cheat sheet can't drift from the real keymap.
pub(crate) fn workspace_keys() -> Vec<KeyBinding> {
    let mut keys: Vec<KeyBinding> = shortcuts::SHORTCUT_SPECS.iter().filter_map(|spec| spec.bind.map(|bind| bind(spec.keys))).collect();
    // Inputs bind cmd-f to their own Search action and cmd-shift-f to
    // Replace, swallowing both when not `searchable` — either would shadow
    // the workspace binding whenever the composer or find input is focused.
    // Registering later in the same context wins, so these keep Cmd-F
    // opening the find bar and Cmd-Shift-F opening global search.
    keys.push(KeyBinding::new("cmd-f", FindInChat, Some("Input")));
    keys.push(KeyBinding::new("cmd-shift-f", SearchAllChats, Some("Input")));
    // Message navigation aliases beyond the cheat-sheet rows: arrow keys
    // mirror j/k, and `g` arms the `gg` double-tap (a two-stroke "g g"
    // binding would hold a typed g for the pending-input timeout — the
    // double-tap in `msg_nav` keeps typing instant). All propagate when an
    // input owns the keys (see `msg_nav::nav_keys_allowed`).
    keys.push(KeyBinding::new("up", MsgNavUp, Some("workspace")));
    keys.push(KeyBinding::new("down", MsgNavDown, Some("workspace")));
    keys.push(KeyBinding::new("g", MsgNavTop, Some("workspace")));
    keys
}

/// App-level action handlers. These are global listeners, so menu and dock
/// actions keep working when no window is open — window-level `on_action`
/// handlers only exist while a workspace is mounted.
pub(crate) fn install_app_actions(cx: &mut App) {
    cx.on_action(|_: &QuitApp, cx| lifecycle::request_quit(cx));
    cx.on_action(|_: &NewWindow, cx| lifecycle::open_new_window(cx));
    cx.on_action(|_: &OpenProject, cx| lifecycle::prompt_open_project(cx));
    cx.on_action(|_: &AboutApp, cx| lifecycle::show_about(cx));
    cx.on_action(|_: &CheckForUpdates, cx| update::check_for_updates(cx));
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(false));
}

/// Extra workspace bindings that aren't in the cheat-sheet table: the
/// terminal (Cmd-`, Ctrl-` fallback — macOS may claim Cmd-` for window
/// cycling) and the plan panel (Cmd-Shift-P).
pub(crate) fn panel_keys() -> [KeyBinding; 3] {
    [
        KeyBinding::new("cmd-`", ToggleTerminal, Some("workspace")),
        KeyBinding::new("ctrl-`", ToggleTerminal, Some("workspace")),
        KeyBinding::new("cmd-shift-p", TogglePlan, Some("workspace")),
    ]
}

/// Menus + dock menu — kept beside the keymap so the chrome wiring is one
/// place.
pub(crate) fn install_chrome(cx: &mut App) {
    cx.set_menus(menus::app_menus());
    cx.set_dock_menu(vec![MenuItem::action("New Window", NewWindow)]);
}
