//! The workspace key-context action handlers — split from `root.rs` for the
//! SLOC cap. `workspace_actions` attaches every `on_action` listener the
//! workspace root div carries; the render body owns layout only.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;
use crate::{
    Chat1, Chat2, Chat3, Chat4, Chat5, Chat6, Chat7, Chat8, Chat9, CloseWindow, CopyTranscript, DeleteChat, EmojiPalette, EnterFullscreen,
    EscapeKey, FindInChat, GoToFile, MinimizeWindow, NewChat, OpenPalette, OpenSettings, RecallLast, RecallNext, RecallPrev, RevealChats,
    SearchAllChats, SearchChat, ShortcutsHelp, ThemeDark, ThemeLight, ToggleAgents, ToggleChanges, ToggleDictation, ToggleExplorer,
    TogglePlan, ToggleSidebar, ToggleSnapshots, ToggleTerminal, ViewLogs, ZoomWindow,
};

/// Every `on_action` listener on the workspace root — chat switching, panel
/// toggles, window chrome, transcript actions, recall, and the shell-run
/// bridge. Order matters only for same-action duplicates (there are none).
pub(crate) fn workspace_actions(root: Div, window: &Window, cx: &mut Context<Workspace>) -> Div {
    let ws_new = cx.entity();
    let ws_del = cx.entity();
    let ws_side = cx.entity();
    let ws_agents = cx.entity();
    let ws_palette = cx.entity();
    let ws_find = cx.entity();
    root.on_action(move |_: &NewChat, _, cx| {
        ws_new.update(cx, |this, cx| this.new_chat(cx));
    })
    .on_action(move |_: &DeleteChat, window, cx| {
        ws_del.update(cx, |this, cx| this.delete_chat(this.active, window, cx));
    })
    .on_action(chat_switch::<Chat1>(cx))
    .on_action(chat_switch::<Chat2>(cx))
    .on_action(chat_switch::<Chat3>(cx))
    .on_action(chat_switch::<Chat4>(cx))
    .on_action(chat_switch::<Chat5>(cx))
    .on_action(chat_switch::<Chat6>(cx))
    .on_action(chat_switch::<Chat7>(cx))
    .on_action(chat_switch::<Chat8>(cx))
    .on_action(chat_switch::<Chat9>(cx))
    .on_action(move |_: &ToggleSidebar, _, cx| {
        ws_side.update(cx, |this, cx| this.toggle_sidebar(cx));
    })
    .on_action(move |_: &ToggleAgents, _, cx| {
        ws_agents.update(cx, |this, cx| this.toggle_agents_panel(cx));
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ToggleChanges, _, cx| {
            ws.update(cx, |this, cx| this.toggle_changes_panel(cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ToggleSnapshots, _, cx| {
            ws.update(cx, |this, cx| this.toggle_snapshots_panel(cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &TogglePlan, _, cx| {
            ws.update(cx, |this, cx| this.toggle_plan_panel(cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ToggleExplorer, _, cx| {
            ws.update(cx, |this, cx| this.toggle_explorer(cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ToggleTerminal, window, cx| {
            ws.update(cx, |this, cx| this.toggle_terminal(window, cx));
        }
    })
    .on_action(move |_: &OpenPalette, window, cx| {
        ws_palette.update(cx, |this, cx| this.open_palette(window, cx));
    })
    .on_action({
        let ws = cx.entity();
        move |_: &GoToFile, window, cx| {
            ws.update(cx, |this, cx| this.open_file_palette(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ThemeLight, window, cx| {
            ws.update(cx, |this, cx| this.set_theme("light", window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ThemeDark, window, cx| {
            ws.update(cx, |this, cx| this.set_theme("dark", window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        let handle = window.window_handle();
        move |_: &CloseWindow, window, cx| crate::window::close_window(&ws, handle, window, cx)
    })
    .on_action({
        let ws = cx.entity();
        move |_: &OpenSettings, window, cx| {
            ws.update(cx, |this, cx| this.open_settings(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &SearchChat, window, cx| {
            ws.update(cx, |this, cx| this.open_chat_search(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &SearchAllChats, window, cx| {
            ws.update(cx, |this, cx| this.open_global_search(window, cx));
        }
    })
    // Reached only when focus is outside the chat column — the column's own
    // FindInChat listener consumes it first.
    .on_action(move |_: &FindInChat, window, cx| ws_find.update(cx, |this, cx| this.open_chat_find(window, cx)))
    .on_action(|_: &MinimizeWindow, window, _cx| window.minimize_window())
    .on_action(|_: &ZoomWindow, window, _cx| window.zoom_window())
    .on_action(|_: &EnterFullscreen, window, _cx| window.toggle_fullscreen())
    .on_action(|_: &EmojiPalette, window, _cx| window.show_character_palette())
    .on_action(|_: &RevealChats, _window, cx| cx.reveal_path(&crate::persist::chats_dir()))
    .on_action({
        let ws = cx.entity();
        move |_: &CopyTranscript, _window, cx| {
            ws.update(cx, |this, cx| this.copy_transcript(cx));
        }
    })
    .on_action(escape_key(cx.entity()))
    .on_action({
        let ws = cx.entity();
        move |_: &ShortcutsHelp, window, cx| {
            ws.update(cx, |this, cx| this.shortcuts_help(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ViewLogs, window, cx| {
            ws.update(cx, |this, cx| this.toggle_logs(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &RecallLast, window, cx| {
            ws.update(cx, |this, cx| this.recall_last(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &RecallPrev, window, cx| {
            ws.update(cx, |this, cx| this.recall_prev(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &RecallNext, window, cx| {
            ws.update(cx, |this, cx| this.recall_next(window, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |action: &crate::run_cmd::RunShellCommand, _window, cx| {
            ws.update(cx, |this, cx| this.run_command_block(action.command.clone(), action.shell, cx));
        }
    })
    .on_action({
        let ws = cx.entity();
        move |_: &ToggleDictation, window, cx| {
            ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
        }
    })
}

/// Esc: the lightbox sits above every other layer, so it dismisses first;
/// then the logs overlay; otherwise the workspace's usual Esc cascade runs.
fn escape_key(ws: Entity<Workspace>) -> impl Fn(&EscapeKey, &mut Window, &mut App) + 'static {
    move |_: &EscapeKey, window, cx| {
        if ws.read(cx).image_view.is_some() {
            ws.update(cx, |this, cx| this.close_image_view(cx));
        } else if ws.read(cx).logs_open {
            ws.update(cx, |this, cx| {
                this.logs_open = false;
                cx.notify();
            });
        } else {
            ws.update(cx, |this, cx| this.escape(window, cx));
        }
    }
}

/// Build an `on_action` handler that selects the chat at sidebar position
/// `A::IX` — matching the visible order (pinned first, then recency).
fn chat_switch<A: Action + ChatIx>(cx: &mut Context<Workspace>) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let ws = cx.entity();
    move |_: &A, window, cx| {
        ws.update(cx, |this, cx| {
            let query = this.search.read(cx).value().to_lowercase();
            if let Some(&ix) = this.sidebar_order(&query).get(A::IX) {
                this.select_chat(ix, window, cx);
            }
        });
    }
}

trait ChatIx {
    const IX: usize;
}
macro_rules! chat_ix {
    ($($t:ident => $n:literal),*) => { $(impl ChatIx for $t { const IX: usize = $n; })* };
}
chat_ix!(Chat1 => 0, Chat2 => 1, Chat3 => 2, Chat4 => 3, Chat5 => 4, Chat6 => 5, Chat7 => 6, Chat8 => 7, Chat9 => 8);
