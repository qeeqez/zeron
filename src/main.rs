mod agents;
mod backend;
mod backend_parse;
mod backend_run;
mod chat_msg;
mod chat_ops;
mod export;
mod files;
mod lifecycle;
mod model;
mod palette;
mod persist;
mod root;
mod send;
mod simulate;
mod views;
mod window;
mod workspace;

use gpui_kit::*;

actions!([
    NewChat, DeleteChat, ToggleSidebar, ToggleAgents, OpenPalette, ThemeLight, ThemeDark, Chat1, Chat2, Chat3, Chat4, Chat5, Chat6, Chat7,
    Chat8, Chat9, CloseWindow, QuitApp, OpenSettings, SearchChat, CopyTranscript, EmojiPalette, RevealChats, EscapeKey, ShortcutsHelp,
    RecallLast, RecallPrev, RecallNext,
]);

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::Assets::new("")).run(|cx| {
        gpui_kit::init(cx);
        cx.set_menus([
            gpui_kit::Menu::new("Rixl Code").items([
                gpui_kit::MenuItem::action("About Rixl Code", gpui_kit::NoAction),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Settings…", OpenSettings),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Quit Rixl Code", QuitApp),
            ]),
            gpui_kit::Menu::new("File").items([
                gpui_kit::MenuItem::action("New Chat", NewChat),
                gpui_kit::MenuItem::action("Reveal Chats Folder", RevealChats),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Close Window", CloseWindow),
            ]),
            gpui_kit::Menu::new("Edit").items([
                gpui_kit::MenuItem::os_action("Cut", gpui_kit::NoAction, gpui_kit::OsAction::Cut),
                gpui_kit::MenuItem::os_action("Copy", gpui_kit::NoAction, gpui_kit::OsAction::Copy),
                gpui_kit::MenuItem::os_action("Paste", gpui_kit::NoAction, gpui_kit::OsAction::Paste),
                gpui_kit::MenuItem::os_action("Select All", gpui_kit::NoAction, gpui_kit::OsAction::SelectAll),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Copy Transcript", CopyTranscript),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Emoji & Symbols", EmojiPalette),
            ]),
            gpui_kit::Menu::new("View").items([
                gpui_kit::MenuItem::action("Toggle Sidebar", ToggleSidebar),
                gpui_kit::MenuItem::action("Toggle Agents", ToggleAgents),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Command Palette", OpenPalette),
            ]),
        ]);
        cx.bind_keys([
            KeyBinding::new("escape", EscapeKey, Some("workspace")),
            KeyBinding::new("cmd-/", ShortcutsHelp, Some("workspace")),
            KeyBinding::new("cmd-n", NewChat, Some("workspace")),
            KeyBinding::new("cmd-,", OpenSettings, Some("workspace")),
            KeyBinding::new("cmd-up", RecallLast, Some("workspace")),
            KeyBinding::new("cmd-shift-up", RecallPrev, Some("workspace")),
            KeyBinding::new("cmd-shift-backspace", DeleteChat, Some("workspace")),
            KeyBinding::new("cmd-shift-down", RecallNext, Some("workspace")),
            KeyBinding::new("cmd-j", ToggleAgents, Some("workspace")),
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
        ]);
        cx.spawn(async move |cx| {
            root::open_workspace_window(cx).expect("failed to open window");
        })
        .detach();
    });
}
