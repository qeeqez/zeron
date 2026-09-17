use gpui_kit::component::IndexPath;
use gpui_kit::component::WindowExt;
use gpui_kit::component::command::Command;
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

use crate::palette_items::Entry;
use crate::workspace::Workspace;

impl Workspace {
    /// Cmd+K: fuzzy command palette — commands plus chat navigation, like
    /// Codex's. Pressing it again (or with any dialog up) closes the dialog.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
            return;
        }
        // Fresh query each open — the state entity persists across dialogs.
        self.palette.update(cx, |state, cx| state.set_query("", window, cx));
        // Snapshot chats now: the dialog builder runs while `render` holds
        // the workspace lease, so it can't read `self` — it rebuilds groups
        // from this snapshot + the live query on every render.
        let chats = self.palette_chats();
        // Same snapshot rule as `chats`: the gated "Stop All Replies" row is
        // baked in at open — a turn ending mid-dialog doesn't shift rows.
        let running = self.running_chats();
        let palette = self.palette.clone();
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .close_button(false)
                .overlay_closable(true)
                .child(palette_command(&palette, &chats, running, &ws, cx))
        });
        // The dialog focuses its own handle on open; the palette needs its
        // query field focused so typing and ↑↓/Enter reach the Command
        // context. Synchronous: it runs after open_dialog's focus, so the
        // input wins; the node registers on the next draw.
        self.palette.update(cx, |state, cx| state.focus(window, cx));
    }

    pub fn open_rename(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        let title = chat.title.clone();
        // Store the stable id — vec positions shift if chats are deleted
        // while the dialog is open. Dialog mode keeps the sidebar row from
        // mounting its inline editor on the same input state.
        self.renaming = Some(chat.id);
        self.rename_mode = crate::workspace::RenameMode::Dialog;
        self.rename.update(cx, |state, cx| {
            state.set_value(title, window, cx);
        });
        let ws = cx.entity();
        // Captured now — the builder runs during render while the workspace
        // is leased, so it can't `ws.read` for the input state.
        let input = self.rename.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            dialog
                .title("Rename chat")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_rename(window, cx));
                    true
                })
                .on_cancel({
                    let ws = ws.clone();
                    move |_, _, cx| cancel_rename(&ws, cx)
                })
        });
    }

    pub fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.rename_mode;
        let Some(id) = self.renaming.take() else { return };
        // An inline commit leaves focus on the now-hidden editor — hand it
        // back to the composer. Dialog mode restores focus on close itself.
        if mode == crate::workspace::RenameMode::Inline {
            self.composer.update(cx, |s, cx| s.focus(window, cx));
        }
        let title = self.rename.read(cx).value().trim().to_string();
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == id);
        if !title.is_empty()
            && let Some(chat) = self.chats.iter_mut().find(|c| c.id == id)
        {
            chat.title = title.into();
            chat.title_custom = true;
            if is_active {
                window.set_window_title(&format!("{} — Rixl Code", chat.title));
            }
        }
        cx.notify();
        self.save();
    }

    /// Open the Codex-style settings screen — a full-window overlay with a
    /// nav rail + content pane, owned by `SettingsPanel`.
    pub fn open_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        cx.notify();
    }

    /// Esc: stop a running reply, close chat search, close the side panels.
    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The modal cheat sheet dismisses first — it sits above everything.
        if self.shortcuts_open {
            self.shortcuts_open = false;
            cx.notify();
            return;
        }
        if self.chats[self.active].running {
            self.stop_reply(cx);
            return;
        }
        if self.chat_search_open {
            self.open_chat_search(window, cx);
            return;
        }
        if self.settings_open {
            self.settings_open = false;
            cx.notify();
            return;
        }
        // A sidebar multi-selection drops next — it's a lighter state than
        // the side panels, so Esc peels it before they close. When the
        // sidebar holds the keyboard (a Cmd-click put it there), focus goes
        // back to the composer too so the next keystroke isn't stranded.
        if !self.selected_chats.is_empty() {
            self.selected_chats.clear();
            if self.sidebar_focus.is_focused(window) {
                self.composer.update(cx, |s, cx| s.focus(window, cx));
            }
            cx.notify();
            return;
        }
        if self.agents_panel_open {
            self.agents_panel_open = false;
            cx.notify();
        }
        if self.changes_panel_open {
            self.changes_panel_open = false;
            cx.notify();
        }
        // Nothing else to peel — if the sidebar still holds the keyboard
        // (Cmd-clicked a row, then Esc dropped the selection), hand focus
        // back to the composer.
        if self.sidebar_focus.is_focused(window) {
            self.composer.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }

    /// Cmd-/: toggle the keyboard-shortcuts cheat sheet — a centered overlay
    /// rendered by `Workspace::render` while `shortcuts_open` is set.
    pub fn shortcuts_help(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.shortcuts_open = !self.shortcuts_open;
        cx.notify();
    }
}

impl Workspace {
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.save_settings();
        cx.notify();
    }

    pub fn toggle_agents_panel(&mut self, cx: &mut Context<Self>) {
        self.agents_panel_open = !self.agents_panel_open;
        cx.notify();
    }

    /// Toggle the Changes panel; opening refreshes the change list so the
    /// first render never shows stale rows.
    pub fn toggle_changes_panel(&mut self, cx: &mut Context<Self>) {
        self.changes_panel_open = !self.changes_panel_open;
        if self.changes_panel_open {
            self.refresh_changes(cx);
        }
        cx.notify();
    }
}

fn cancel_rename(ws: &Entity<Workspace>, cx: &mut App) -> bool {
    ws.update(cx, |this, _cx| this.renaming = None);
    true
}

/// The palette's `Command` element, rebuilt by the dialog layer on every
/// workspace render — `on_query` notifies the workspace so each keystroke
/// re-runs this builder with fresh groups for the new query.
fn palette_command(
    palette: &Entity<gpui_kit::component::command::CommandState>, chats: &[crate::palette_items::ChatSnapshot], running: usize,
    ws: &Entity<Workspace>, cx: &mut App,
) -> Command {
    let ws_confirm = ws.clone();
    let ws_query = ws.clone();
    let (commands, chat_group) = crate::palette_items::palette_groups(chats, &palette.read(cx).query(cx), running);
    Command::new(palette)
        .placeholder("Type a command or search chats…")
        // Local filtering is substring-only; ranking is fuzzy and happens
        // in `palette_items`.
        .filterable(false)
        .group(commands)
        .group(chat_group)
        .empty(|_, _, cx| {
            div()
                .py_6()
                .w_full()
                .text_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No matching commands or chats")
        })
        .footer(|_, _, cx| command_footer("↵ select", cx))
        // The dialog builder re-runs on workspace renders — notifying
        // rebuilds the list for the new query.
        .on_query(move |_, _, cx| {
            ws_query.update(cx, |_, cx| cx.notify());
        })
        .on_confirm({
            let ws = ws_confirm.clone();
            move |path, window, cx| {
                ws.update(cx, |this, cx| this.confirm_palette_entry(path, window, cx));
            }
        })
        .on_cancel(|window, cx| window.close_dialog(cx))
}

impl Workspace {
    /// Resolve a confirmed palette row to its entry and run it: commands
    /// execute their effect, chats switch. Runs after the dialog closes so
    /// commands that open their own surface land on the right layer.
    fn confirm_palette_entry(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let query = self.palette.read(cx).query(cx);
        match crate::palette_items::entry_at(&self.palette_chats(), &query, path, self.running_chats()) {
            Some(Entry::Command(spec)) => {
                if let crate::palette_items::Effect::Run(run) = spec.effect {
                    run(self, window, cx);
                }
            },
            Some(Entry::Chat(chat)) => {
                if let Some(ix) = self.chat_index(chat.id) {
                    self.select_chat(ix, window, cx);
                }
            },
            None => {},
        }
    }
}

/// The key-hint strip under a `Command` dialog's list — shared by the
/// palette and global search so both footers stay identical.
pub(crate) fn command_footer(confirm: &'static str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child("↑↓ navigate")
        .child(confirm)
        .child("esc close")
}
