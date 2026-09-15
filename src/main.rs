#![recursion_limit = "1024"]

mod activity;
#[cfg(test)]
mod activity_tests;
mod agents;
mod agents_task;
#[cfg(test)]
mod agents_tests;
mod appearance;
#[cfg(test)]
mod appearance_tests;
mod approval_ops;
#[cfg(test)]
mod approval_tests;
mod attachment;
mod auth;
mod backend;
mod backend_parse;
#[cfg(test)]
mod backend_parse_tests;
mod backend_run;
#[cfg(test)]
mod backend_run_tests;
mod changes;
mod changes_commits;
#[cfg(test)]
mod changes_commits_ui_tests;
mod changes_diff;
#[cfg(test)]
mod changes_diff_tests;
#[cfg(test)]
mod changes_git_ui_tests;
#[cfg(test)]
mod changes_tests;
#[cfg(test)]
mod changes_ui_tests;
mod chat_delete;
mod chat_edit;
#[cfg(test)]
mod chat_edit_tests;
mod chat_find;
#[cfg(test)]
mod chat_find_tests;
mod chat_msg;
mod chat_ops;
#[cfg(test)]
mod chat_ops_tests;
mod chat_search;
mod chat_search_tests;
mod checkpoints;
#[cfg(test)]
mod checkpoints_tests;
#[cfg(test)]
mod composer_queue_tests;
#[cfg(test)]
mod composer_queue_ui_tests;
#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod composer_testutil;
#[cfg(test)]
mod continuity_tests;
mod export;
mod files;
mod git;
mod git_parse;
#[cfg(test)]
mod git_tests;
mod global_search;
#[cfg(test)]
mod global_search_tests;
mod lifecycle;
mod mcp;
mod mcp_config;
mod menus;
mod model;
mod model_catalog;
#[cfg(test)]
mod model_picker_tests;
mod notify;
#[cfg(test)]
mod notify_tests;
mod open_in;
#[cfg(test)]
mod open_in_tests;
mod palette;
mod palette_commands;
mod palette_fuzzy;
mod palette_items;
#[cfg(test)]
mod palette_tests;
mod persist;
mod persist_migrate;
mod persist_model_cache;
#[cfg(test)]
mod persist_tests;
mod project;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod project_ui_tests;
mod provider_ops;
#[cfg(test)]
mod provider_tests;
mod providers;
mod recent_projects;
mod resume;
#[cfg(test)]
mod resume_tests;
mod review;
#[cfg(test)]
mod review_tests;
mod root;
mod send;
mod send_queue;
#[cfg(test)]
mod send_tests;
#[cfg(test)]
mod settings_providers_tests;
mod shortcuts;
#[cfg(test)]
mod shortcuts_tests;
#[cfg(test)]
mod sidebar_resize_tests;
#[cfg(test)]
mod sidebar_ui_tests;
mod simulate;
mod slash;
mod snapshot_store;
mod snapshots;
#[cfg(test)]
mod snapshots_tests;
mod speech;
mod steer;
#[cfg(test)]
mod steer_tests;
mod thread_defaults;
#[cfg(test)]
mod thread_defaults_tests;
#[cfg(test)]
mod ui_tests;
mod usage;
#[cfg(test)]
mod usage_tests;
mod views;
mod voice;
#[cfg(target_os = "macos")]
mod voice_apple;
#[cfg(test)]
mod voice_tests;
mod window;
#[cfg(test)]
mod window_chrome_tests;
mod workspace;
mod workspace_new;
mod workspace_settings;
#[cfg(test)]
mod workspace_tests;
mod worktree;
#[cfg(test)]
mod worktree_tests;

use gpui_kit::*;

actions!([
    NewChat, DeleteChat, ToggleSidebar, ToggleAgents, ToggleChanges, ToggleSnapshots, ToggleExplorer, OpenPalette, ThemeLight, ThemeDark,
    Chat1, Chat2, Chat3, Chat4, Chat5, Chat6, Chat7, Chat8, Chat9, CloseWindow, QuitApp, OpenSettings, SearchChat, SearchAllChats,
    FindInChat, CopyTranscript, EmojiPalette, RevealChats, EscapeKey, ShortcutsHelp, RecallLast, RecallPrev, RecallNext, NewWindow,
    OpenProject, AboutApp, HideApp, HideOthers, MinimizeWindow, ZoomWindow, EnterFullscreen, BringAllToFront, ToggleDictation,
]);

/// Workspace-context key bindings. Menu items pick their key equivalents up
/// from these, so a shortcut added here shows in the menu bar for free. The
/// list comes from `shortcuts::SHORTCUT_SPECS` — the same table the Cmd-/
/// overlay renders, so the cheat sheet can't drift from the real keymap.
fn workspace_keys() -> Vec<KeyBinding> {
    let mut keys: Vec<KeyBinding> = shortcuts::SHORTCUT_SPECS.iter().filter_map(|spec| spec.bind.map(|bind| bind(spec.keys))).collect();
    // Inputs bind cmd-f to their own Search action and cmd-shift-f to
    // Replace, swallowing both when not `searchable` — either would shadow
    // the workspace binding whenever the composer or find input is focused.
    // Registering later in the same context wins, so these keep Cmd-F
    // opening the find bar and Cmd-Shift-F opening global search.
    keys.push(KeyBinding::new("cmd-f", FindInChat, Some("Input")));
    keys.push(KeyBinding::new("cmd-shift-f", SearchAllChats, Some("Input")));
    keys
}

/// App-level action handlers. These are global listeners, so menu and dock
/// actions keep working when no window is open — window-level `on_action`
/// handlers only exist while a workspace is mounted.
fn install_app_actions(cx: &mut App) {
    cx.on_action(|_: &QuitApp, cx| lifecycle::request_quit(cx));
    cx.on_action(|_: &NewWindow, cx| lifecycle::open_new_window(cx));
    cx.on_action(|_: &OpenProject, cx| lifecycle::prompt_open_project(cx));
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
        cx.set_menus(menus::app_menus());
        cx.set_dock_menu(vec![MenuItem::action("New Window", NewWindow)]);
        cx.bind_keys(workspace_keys());
        install_app_actions(cx);
        cx.spawn(async move |cx| {
            lifecycle::open_workspace_window(cx).expect("failed to open window");
        })
        .detach();
    });
}
