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
        self.renaming = Some(ix);
        let title = self.chats[ix].title.clone();
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
                .on_ok(move |_, _, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_rename(cx));
                    true
                })
                .on_cancel({
                    let ws = ws.clone();
                    move |_, _, cx| cancel_rename(&ws, cx)
                })
        });
    }

    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.renaming.take() else { return };
        let title = self.rename.read(cx).value().trim().to_string();
        if !title.is_empty() {
            self.chats[ix].title = title.into();
        }
        cx.notify();
    }
}

fn cancel_rename(ws: &Entity<Workspace>, cx: &mut App) -> bool {
    ws.update(cx, |this, _cx| this.renaming = None);
    true
}

fn palette_items() -> Vec<CommandItem> {
    vec![
        CommandItem::new().label("New Chat").action(Box::new(crate::NewChat)),
        CommandItem::new().label("Toggle Sidebar").action(Box::new(crate::ToggleSidebar)),
        CommandItem::new().label("Toggle Agents Panel").action(Box::new(crate::ToggleAgents)),
        CommandItem::new().label("Switch to Light Theme").action(Box::new(crate::ThemeLight)),
        CommandItem::new().label("Switch to Dark Theme").action(Box::new(crate::ThemeDark)),
    ]
}
