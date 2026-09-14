#![recursion_limit = "1024"]

mod agents;
mod appearance;
#[cfg(test)]
mod appearance_tests;
mod backend;
mod backend_parse;
#[cfg(test)]
mod backend_parse_tests;
mod backend_run;
mod changes_diff;
#[cfg(test)]
mod changes_diff_tests;
#[cfg(test)]
mod changes_ui_tests;
mod chat_delete;
mod chat_msg;
mod chat_ops;
mod chat_search;
mod chat_search_tests;
#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod continuity_tests;
mod export;
mod files;
mod git;
#[cfg(test)]
mod git_tests;
mod lifecycle;
mod model;
mod notify;
#[cfg(test)]
mod notify_tests;
mod palette;
mod palette_fuzzy;
mod palette_items;
#[cfg(test)]
mod palette_tests;
mod persist;
mod persist_tests;
mod project;
#[cfg(test)]
mod project_tests;
mod root;
mod send;
mod simulate;
#[cfg(test)]
mod ui_tests;
mod views;
mod window;
#[cfg(test)]
mod window_chrome_tests;
mod workspace;

use gpui_kit::*;

actions!([
    NewChat, DeleteChat, ToggleSidebar, ToggleAgents, ToggleChanges, OpenPalette, ThemeLight, ThemeDark, Chat1, Chat2, Chat3, Chat4, Chat5,
    Chat6, Chat7, Chat8, Chat9, CloseWindow, QuitApp, OpenSettings, SearchChat, CopyTranscript, EmojiPalette, RevealChats, EscapeKey,
    ShortcutsHelp, RecallLast, RecallPrev, RecallNext, NewWindow, AboutApp, HideApp, HideOthers, MinimizeWindow, ZoomWindow,
    EnterFullscreen, BringAllToFront,
]);

/// The macOS menu bar. Menu actions dispatch to the active window (or the
/// global listeners installed in `main` when no window is open); key
/// equivalents come from `workspace_keys` via the keymap. A menu named
/// "Window" is registered with AppKit as the system window menu.
fn app_menus() -> Vec<Menu> {
    use gpui_kit::component::input;
    [
        Menu::new("Rixl Code").items([
            MenuItem::action("About Rixl Code", AboutApp),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Rixl Code", HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::separator(),
            MenuItem::action("Quit Rixl Code", QuitApp),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Chat", NewChat),
            MenuItem::action("New Window", NewWindow),
            MenuItem::separator(),
            MenuItem::action("Reveal Chats Folder", RevealChats),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", input::Undo),
            MenuItem::action("Redo", input::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Copy Transcript", CopyTranscript),
            MenuItem::separator(),
            MenuItem::action("Emoji & Symbols", EmojiPalette),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Agents", ToggleAgents),
            MenuItem::action("Toggle Changes", ToggleChanges),
            MenuItem::separator(),
            MenuItem::action("Command Palette", OpenPalette),
            MenuItem::separator(),
            MenuItem::action("Enter Full Screen", EnterFullscreen),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ]),
    ]
    .into()
}

/// Workspace-context key bindings. Menu items pick their key equivalents up
/// from these, so a shortcut added here shows in the menu bar for free.
fn workspace_keys() -> Vec<KeyBinding> {
    [
        KeyBinding::new("escape", EscapeKey, Some("workspace")),
        KeyBinding::new("cmd-/", ShortcutsHelp, Some("workspace")),
        KeyBinding::new("cmd-n", NewChat, Some("workspace")),
        KeyBinding::new("cmd-shift-n", NewWindow, Some("workspace")),
        KeyBinding::new("cmd-,", OpenSettings, Some("workspace")),
        KeyBinding::new("cmd-q", QuitApp, Some("workspace")),
        KeyBinding::new("cmd-h", HideApp, Some("workspace")),
        KeyBinding::new("cmd-alt-h", HideOthers, Some("workspace")),
        KeyBinding::new("cmd-m", MinimizeWindow, Some("workspace")),
        KeyBinding::new("ctrl-cmd-f", EnterFullscreen, Some("workspace")),
        KeyBinding::new("cmd-up", RecallLast, Some("workspace")),
        KeyBinding::new("cmd-shift-up", RecallPrev, Some("workspace")),
        KeyBinding::new("cmd-shift-backspace", DeleteChat, Some("workspace")),
        KeyBinding::new("cmd-shift-down", RecallNext, Some("workspace")),
        KeyBinding::new("cmd-j", ToggleAgents, Some("workspace")),
        KeyBinding::new("cmd-shift-j", ToggleChanges, Some("workspace")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("workspace")),
        KeyBinding::new("cmd-k", OpenPalette, Some("workspace")),
        KeyBinding::new("cmd-w", CloseWindow, Some("workspace")),
        KeyBinding::new("cmd-f", SearchChat, Some("workspace")),
        KeyBinding::new("cmd-1", Chat1, Some("workspace")),
        KeyBinding::new("cmd-2", Chat2, Some("workspace")),
        KeyBinding::new("cmd-3", Chat3, Some("workspace")),
        KeyBinding::new("cmd-4", Chat4, Some("workspace")),
        KeyBinding::new("cmd-5", Chat5, Some("workspace")),
        KeyBinding::new("cmd-6", Chat6, Some("workspace")),
        KeyBinding::new("cmd-7", Chat7, Some("workspace")),
        KeyBinding::new("cmd-8", Chat8, Some("workspace")),
        KeyBinding::new("cmd-9", Chat9, Some("workspace")),
    ]
    .into()
}

/// App-level action handlers. These are global listeners, so menu and dock
/// actions keep working when no window is open — window-level `on_action`
/// handlers only exist while a workspace is mounted.
fn install_app_actions(cx: &mut App) {
    cx.on_action(|_: &QuitApp, cx| lifecycle::request_quit(cx));
    cx.on_action(|_: &NewWindow, cx| lifecycle::open_new_window(cx));
    cx.on_action(|_: &AboutApp, cx| lifecycle::show_about(cx));
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(false));
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets::new(""));
    // Dock click with no visible windows re-opens a workspace — the standard
    // macOS behavior for an app that stays running after its windows close.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            lifecycle::open_new_window(cx);
        }
    });
    app.run(|cx| {
        gpui_kit::init(cx);
        // Names the app in system notifications on platforms that need an
        // explicit identity (Linux/Windows); a no-op on macOS, where the
        // bundle provides it.
        cx.set_app_identity("com.rixl.rixlcode", "Rixl Code");
        cx.set_menus(app_menus());
        cx.set_dock_menu(vec![MenuItem::action("New Window", NewWindow)]);
        cx.bind_keys(workspace_keys());
        install_app_actions(cx);
        cx.spawn(async move |cx| {
            root::open_workspace_window(cx).expect("failed to open window");
        })
        .detach();
    });
}
