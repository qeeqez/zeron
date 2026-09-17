//! The command-palette's command table — Codex's palette mixes app commands
//! with chat jumps. Order is the empty-query display order; a query re-sorts
//! by fuzzy score (see `palette_items::build_entries`).

use gpui_kit::assets::IconName;

use crate::palette_items::{CommandSpec, Effect};
use crate::workspace::Workspace;

pub(crate) fn command_specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            label: "New Chat",
            icon: IconName::Plus,
            keywords: &["create", "thread"],
            effect: Effect::Dispatch(Box::new(crate::NewChat)),
        },
        CommandSpec {
            label: "Open Project…",
            icon: IconName::FolderOpen,
            keywords: &["folder", "switch", "directory"],
            effect: Effect::Dispatch(Box::new(crate::OpenProject)),
        },
        CommandSpec {
            label: "Go to File…",
            icon: IconName::File,
            keywords: &["open", "jump", "path"],
            // Run (not Dispatch): the picker is itself a dialog, so it must
            // open after the palette closes — a deferred dispatch would
            // land while it's still up and just close it.
            effect: Effect::Run(Workspace::open_file_palette),
        },
        CommandSpec {
            label: "Rename Chat",
            icon: IconName::Pencil,
            keywords: &["title"],
            effect: Effect::Run(Workspace::rename_active),
        },
        CommandSpec {
            label: "Delete Chat",
            icon: IconName::Delete,
            keywords: &["remove"],
            effect: Effect::Dispatch(Box::new(crate::DeleteChat)),
        },
        CommandSpec {
            label: "Search in Chat",
            icon: IconName::Search,
            keywords: &["find"],
            effect: Effect::Run(Workspace::open_chat_search),
        },
        CommandSpec {
            label: "Search All Chats",
            icon: IconName::TextSearch,
            keywords: &["find", "messages", "global"],
            effect: Effect::Run(Workspace::open_global_search),
        },
        CommandSpec {
            label: "Copy Transcript",
            icon: IconName::Copy,
            keywords: &["clipboard"],
            effect: Effect::Dispatch(Box::new(crate::CopyTranscript)),
        },
        CommandSpec {
            label: "Export Transcript…",
            icon: IconName::Share,
            keywords: &["markdown", "save"],
            effect: Effect::Run(|this, _window, cx| this.export_active(cx)),
        },
        CommandSpec {
            label: "Export HTML…",
            icon: IconName::FileCode,
            keywords: &["html", "save", "print"],
            effect: Effect::Run(|this, _window, cx| this.export_active_html(cx)),
        },
        CommandSpec {
            label: "Toggle Sidebar",
            icon: IconName::PanelLeft,
            keywords: &[],
            effect: Effect::Dispatch(Box::new(crate::ToggleSidebar)),
        },
        CommandSpec {
            label: "Toggle Agents Panel",
            icon: IconName::Bot,
            keywords: &["tasks"],
            effect: Effect::Dispatch(Box::new(crate::ToggleAgents)),
        },
        CommandSpec {
            label: "Toggle Changes Panel",
            icon: IconName::FileDiff,
            keywords: &["git", "diff"],
            effect: Effect::Dispatch(Box::new(crate::ToggleChanges)),
        },
        CommandSpec {
            label: "Open Settings",
            icon: IconName::Settings,
            keywords: &["preferences"],
            effect: Effect::Dispatch(Box::new(crate::OpenSettings)),
        },
        CommandSpec {
            label: "Reveal Chats Folder",
            icon: IconName::FolderOpen,
            keywords: &["finder"],
            effect: Effect::Dispatch(Box::new(crate::RevealChats)),
        },
        CommandSpec {
            label: "Keyboard Shortcuts",
            icon: IconName::Keyboard,
            keywords: &["help", "keys"],
            effect: Effect::Run(Workspace::shortcuts_help),
        },
        CommandSpec {
            label: "View Logs",
            icon: IconName::ScrollText,
            keywords: &["debug", "diagnostics", "errors"],
            effect: Effect::Dispatch(Box::new(crate::ViewLogs)),
        },
        CommandSpec {
            label: "Usage Panel",
            icon: IconName::ChartPie,
            keywords: &["tokens", "cost", "spend", "dashboard"],
            // Run (not Dispatch): the panel needs no action — the flag
            // flips directly, and no keybinding hints at it.
            effect: Effect::Run(Workspace::toggle_usage_panel),
        },
        CommandSpec {
            label: "Switch to Light Theme",
            icon: IconName::Sun,
            keywords: &["appearance"],
            effect: Effect::Dispatch(Box::new(crate::ThemeLight)),
        },
        CommandSpec {
            label: "Switch to Dark Theme",
            icon: IconName::Moon,
            keywords: &["appearance"],
            effect: Effect::Dispatch(Box::new(crate::ThemeDark)),
        },
    ]
}
