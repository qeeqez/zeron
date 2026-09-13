use gpui_kit::component::WindowExt;
use gpui_kit::component::command::{Command, CommandItem};
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let palette = ws.read(cx).palette.clone();
            dialog.close_button(false).overlay_closable(true).child(
                Command::new(&palette)
                    .placeholder("Type a command…")
                    .items(palette_items())
                    .on_cancel(|window, cx| window.close_dialog(cx)),
            )
        });
    }

    pub fn open_rename(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        let title = chat.title.clone();
        // Store the stable id — vec positions shift if chats are deleted
        // while the dialog is open.
        self.renaming = Some(chat.id);
        self.rename.update(cx, |state, cx| {
            state.set_value(title, window, cx);
        });
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let input = ws.read(cx).rename.clone();
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
        let Some(id) = self.renaming.take() else { return };
        let title = self.rename.read(cx).value().trim().to_string();
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == id);
        if !title.is_empty()
            && let Some(chat) = self.chats.iter_mut().find(|c| c.id == id)
        {
            chat.title = title.into();
            if is_active {
                window.set_window_title(&format!("Rixl Code — {}", chat.title));
            }
        }
        cx.notify();
        self.save();
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = cx.entity();
        window.open_sheet(cx, move |sheet, _window, cx| {
            let panel = cx.new(|cx| crate::views::settings::SettingsPanel::new(ws.clone(), cx));
            sheet.title("Settings").child(panel)
        });
    }

    pub fn open_chat_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_search_open = !self.chat_search_open;
        self.search_match_ix = 0;
        if self.chat_search_open {
            let input = self.chat_search.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        } else {
            self.chat_search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    }

    /// Esc: stop a running reply, close chat search, close agents panel.
    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats[self.active].running {
            self.stop_reply(cx);
            return;
        }
        if self.chat_search_open {
            self.open_chat_search(window, cx);
            return;
        }
        if self.agents_panel_open {
            self.agents_panel_open = false;
            cx.notify();
        }
    }

    /// Indexes of messages matching the chat-search query.
    fn match_indexes(&self, cx: &App) -> Vec<usize> {
        let query = self.chat_search.read(cx).value().to_lowercase();
        if query.is_empty() {
            return vec![];
        }
        self.chats[self.active]
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                let text = match &m.kind {
                    crate::model::MessageKind::Text(t) => t.as_str(),
                    crate::model::MessageKind::Tool(t) => t.name.as_str(),
                    crate::model::MessageKind::Diff(d) => d.path.as_str(),
                };
                text.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Enter in chat search: jump to next match; Shift+Enter: previous.
    /// `search_match_ix` is the position within the filtered list, which is
    /// what the scroller indexes.
    pub fn jump_to_match(&mut self, back: bool, cx: &mut Context<Self>) {
        let matches = self.match_indexes(cx);
        if matches.is_empty() {
            return;
        }
        self.search_match_ix = if back {
            self.search_match_ix.checked_sub(1).unwrap_or(matches.len() - 1)
        } else {
            (self.search_match_ix + 1) % matches.len()
        };
        self.scroller.update(cx, |s, cx| {
            s.scroll_to_item(self.search_match_ix, cx);
        });
        cx.notify();
    }

    /// Cmd-/: keyboard shortcut cheat sheet.
    pub fn shortcuts_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_sheet(cx, |sheet, _window, _cx| {
            sheet.title("Keyboard Shortcuts").child(div().flex().flex_col().gap_1().p_4().text_xs().children(
                crate::views::settings::SHORTCUTS.iter().map(|(key, desc)| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(160.)).font_weight(FontWeight::SEMIBOLD).child(*key))
                        .child(div().text_color(hsla(0.0, 0.0, 0.55, 1.0)).child(*desc))
                }),
            ))
        });
    }
}

fn cancel_rename(ws: &Entity<Workspace>, cx: &mut App) -> bool {
    ws.update(cx, |this, _cx| this.renaming = None);
    true
}

fn palette_items() -> Vec<CommandItem> {
    vec![
        CommandItem::new().label("New Chat").action(Box::new(crate::NewChat)),
        CommandItem::new().label("Delete Chat").action(Box::new(crate::DeleteChat)),
        CommandItem::new().label("Toggle Sidebar").action(Box::new(crate::ToggleSidebar)),
        CommandItem::new().label("Toggle Agents Panel").action(Box::new(crate::ToggleAgents)),
        CommandItem::new().label("Switch to Light Theme").action(Box::new(crate::ThemeLight)),
        CommandItem::new().label("Switch to Dark Theme").action(Box::new(crate::ThemeDark)),
    ]
}
