//! The macOS menu bar. Menu actions dispatch to the active window (or the
//! global listeners installed in `main` when no window is open); key
//! equivalents come from `workspace_keys` via the keymap. A menu named
//! "Window" is registered with AppKit as the system window menu; "Help"
//! comes last per the macOS convention.

use gpui_kit::*;

use crate::{
    AboutApp, BringAllToFront, CheckForUpdates, CloseWindow, CopyTranscript, EmojiPalette, EnterFullscreen, FindInChat, HideApp,
    HideOthers, MinimizeWindow, NewChat, NewWindow, OpenPalette, OpenProject, OpenSettings, QuitApp, ReleaseNotes, ReportIssue,
    RevealChats, RevealLogs, SearchAllChats, ShortcutsHelp, ShowAll, ToggleAgents, ToggleChanges, ToggleDictation, ToggleExplorer,
    TogglePlan, ToggleSidebar, ToggleSnapshots, ToggleTerminal, ZoomWindow,
};

pub(crate) fn app_menus() -> Vec<Menu> {
    use gpui_kit::component::input;
    [
        Menu::new("Rixl Code").items([
            MenuItem::action("About Rixl Code", AboutApp),
            MenuItem::separator(),
            MenuItem::action("Check for Updates", CheckForUpdates),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Rixl Code", HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Rixl Code", QuitApp),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Chat", NewChat),
            MenuItem::action("New Window", NewWindow),
            MenuItem::action("Open Project…", OpenProject),
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
            MenuItem::action("Find in Chat", FindInChat),
            MenuItem::action("Search All Chats", SearchAllChats),
            MenuItem::separator(),
            MenuItem::action("Start Dictation", ToggleDictation),
            MenuItem::action("Emoji & Symbols", EmojiPalette),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Agents", ToggleAgents),
            MenuItem::action("Toggle Changes", ToggleChanges),
            MenuItem::action("Toggle Plan", TogglePlan),
            MenuItem::action("Toggle Snapshots", ToggleSnapshots),
            MenuItem::action("Toggle Explorer", ToggleExplorer),
            MenuItem::action("Toggle Terminal", ToggleTerminal),
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
        Menu::new("Help").items([
            MenuItem::action("Keyboard Shortcuts", ShortcutsHelp),
            MenuItem::separator(),
            MenuItem::action("Release Notes", ReleaseNotes),
            MenuItem::action("Report an Issue", ReportIssue),
            MenuItem::separator(),
            MenuItem::action("Reveal Logs Folder", RevealLogs),
        ]),
    ]
    .into()
}
