#![recursion_limit = "1024"]

mod activity;
#[cfg(test)]
mod activity_tests;
mod agents;
mod agents_task;
#[cfg(test)]
mod agents_tests;
mod app_icon;
#[cfg(test)]
mod app_icon_tests;
mod app_setup;
mod appearance;
#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod appearance_zoom_tests;
mod apply_code;
#[cfg(test)]
mod apply_code_tests;
#[cfg(test)]
mod apply_code_ui_tests;
mod approval_ops;
#[cfg(test)]
mod approval_tests;
mod attachment;
mod auth;
mod backend;
mod backend_apply;
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
mod changes_stale_tests;
mod changes_stash;
#[cfg(test)]
mod changes_tests;
#[cfg(test)]
mod changes_ui_tests;
mod chat_delete;
mod chat_edit;
#[cfg(test)]
mod chat_edit_tests;
mod chat_fork;
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
#[cfg(test)]
mod diff_mode_tests;
mod export;
mod feedback;
#[cfg(test)]
mod feedback_tests;
mod file_palette;
#[cfg(test)]
mod file_palette_tests;
mod files;
mod git;
mod git_parse;
#[cfg(test)]
mod git_tests;
mod global_search;
#[cfg(test)]
mod global_search_tests;
mod image_view;
#[cfg(test)]
mod image_view_tests;
mod instructions;
#[cfg(test)]
mod instructions_tests;
mod lifecycle;
mod logs;
#[cfg(test)]
mod logs_tests;
mod mcp;
mod mcp_config;
mod menus;
mod model;
mod model_catalog;
#[cfg(test)]
mod model_picker_tests;
mod msg_nav;
#[cfg(test)]
mod msg_nav_tests;
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
mod persist_settings;
#[cfg(test)]
mod persist_tests;
mod plan_panel;
#[cfg(test)]
mod plan_panel_tests;
mod pricing;
mod project;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod project_ui_tests;
mod prompts;
#[cfg(test)]
mod prompts_tests;
mod provider_ops;
#[cfg(test)]
mod provider_tests;
mod providers;
mod rate_limit;
#[cfg(test)]
mod rate_limit_tests;
mod recent_projects;
mod resume;
#[cfg(test)]
mod resume_tests;
mod review;
#[cfg(test)]
mod review_tests;
mod root;
mod root_actions;
mod run_cmd;
#[cfg(test)]
mod run_cmd_tests;
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
mod terminal;
#[cfg(test)]
mod terminal_tests;
mod thread_defaults;
#[cfg(test)]
mod thread_defaults_tests;
mod trust;
#[cfg(test)]
mod trust_tests;
#[cfg(test)]
mod ui_tests;
mod update;
mod update_check;
#[cfg(test)]
mod update_tests;
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
    NewChat, DeleteChat, ToggleSidebar, ToggleAgents, ToggleChanges, ToggleSnapshots, ToggleExplorer, TogglePlan, OpenPalette, GoToFile,
    ThemeLight, ThemeDark, Chat1, Chat2, Chat3, Chat4, Chat5, Chat6, Chat7, Chat8, Chat9, CloseWindow, QuitApp, OpenSettings, SearchChat,
    SearchAllChats, FindInChat, CopyTranscript, EmojiPalette, RevealChats, EscapeKey, ShortcutsHelp, RecallLast, RecallPrev, RecallNext,
    NewWindow, OpenProject, AboutApp, CheckForUpdates, HideApp, HideOthers, MinimizeWindow, ZoomWindow, EnterFullscreen, BringAllToFront,
    ToggleDictation, ToggleTerminal, MsgNavDown, MsgNavUp, MsgNavTop, MsgNavBottom, MsgNavEnter, ViewLogs, ZoomIn, ZoomOut, ZoomReset,
]);

// Re-exported at the crate root for tests — they bind the workspace keymap
// and app actions via `crate::workspace_keys()` / `crate::install_app_actions`.
#[cfg(test)]
pub(crate) use app_setup::{install_app_actions, panel_keys, workspace_keys};

fn main() {
    // Capture log output into the in-app ring buffer + log file before
    // anything else can emit records the View Logs panel should show.
    logs::install();
    let app = gpui_kit::application().with_assets(app_icon::AppAssets::new());
    // Dock click with no visible windows re-opens a workspace — the standard
    // macOS behavior for an app that stays running after its windows close.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            lifecycle::open_new_window(cx)
        }
    });
    app.run(|cx| {
        gpui_kit::init(cx);
        // Dock icon — the runtime stand-in for a bundle icon while the app
        // ships unbundled; a real .app's CFBundleIconFile wins (it skips
        // itself when an icon is already set).
        app_icon::install_dock_icon();
        // Names the app in system notifications on platforms that need an
        // explicit identity (Linux/Windows); a no-op on macOS, where the
        // bundle provides it.
        cx.set_app_identity("com.rixl.rixlcode", "Rixl Code");
        app_setup::install_chrome(cx);
        cx.bind_keys(app_setup::workspace_keys());
        cx.bind_keys(app_setup::panel_keys());
        app_setup::install_app_actions(cx);
        lifecycle::spawn_launch_window(cx);
    });
}
