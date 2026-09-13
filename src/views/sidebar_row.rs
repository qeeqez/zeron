use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::sidebar::SidebarMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

pub(super) fn chat_row(chat: &crate::model::Chat, ix: usize, active: usize, cx: &mut Context<Workspace>) -> SidebarMenuItem {
    let running = chat.running;
    let pinned = chat.pinned;
    let unread = chat.unread;
    let archived = chat.archived;
    let id = chat.id;
    let ws = cx.entity();
    SidebarMenuItem::new(chat.title.clone())
        .active(ix == active)
        .icon(if pinned { IconName::StarFill } else { IconName::FileText })
        .suffix(move |_window, _cx| {
            if running {
                IconName::LoaderCircle.into_any_element()
            } else if unread {
                div().w_2().h_2().rounded_full().bg(hsla(0.0, 0.0, 0.55, 1.0)).into_any_element()
            } else {
                div().into_any_element()
            }
        })
        .context_menu(move |menu, _window, _cx| chat_row_menu(&ws, id, RowFlags { pinned, archived }, menu))
        .on_click(cx.listener(move |this, _, window, cx| {
            if let Some(ix) = this.chat_index(id) {
                this.select_chat(ix, window, cx);
            }
        }))
}

/// Per-row state the context menu needs.
pub(super) struct RowFlags {
    pinned: bool,
    archived: bool,
}

pub(super) fn chat_row_menu(ws: &Entity<Workspace>, id: u64, flags: RowFlags, menu: PopupMenu) -> PopupMenu {
    let RowFlags { pinned, archived } = flags;
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_dup = ws.clone();
    let ws_export = ws.clone();
    let ws_del = ws.clone();
    let ws_arch = ws.clone();
    menu.item(PopupMenuItem::new(pin_label).icon(IconName::Star).on_click(move |_, w, cx| {
        with_chat_ix(ChatIxArgs {
            ws: &ws_pin,
            id,
            window: w,
            cx,
            f: |this, ix, _w, cx| this.toggle_pin(ix, cx),
        });
    }))
    .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, w, cx| {
        with_chat_ix(ChatIxArgs {
            ws: &ws_rename,
            id,
            window: w,
            cx,
            f: |this, ix, w, cx| this.open_rename(ix, w, cx),
        });
    }))
    .item(PopupMenuItem::new("Duplicate").icon(IconName::Copy).on_click(move |_, w, cx| {
        with_chat_ix(ChatIxArgs {
            ws: &ws_dup,
            id,
            window: w,
            cx,
            f: |this, ix, w, cx| this.duplicate_chat(ix, w, cx),
        });
    }))
    .item(PopupMenuItem::new("Export").icon(IconName::Share).on_click(move |_, w, cx| {
        with_chat_ix(ChatIxArgs {
            ws: &ws_export,
            id,
            window: w,
            cx,
            f: |this, ix, _w, cx| this.export_chat(ix, cx),
        });
    }))
    .item(PopupMenuItem::new("Delete").icon(IconName::Delete).on_click(move |_, w, cx| {
        with_chat_ix(ChatIxArgs {
            ws: &ws_del,
            id,
            window: w,
            cx,
            f: |this, ix, w, cx| this.delete_chat(ix, w, cx),
        });
    }))
    .item(
        PopupMenuItem::new(if archived { "Unarchive" } else { "Archive" })
            .icon(IconName::Archive)
            .on_click(move |_, w, cx| {
                with_chat_ix(ChatIxArgs {
                    ws: &ws_arch,
                    id,
                    window: w,
                    cx,
                    f: |this, ix, w, cx| this.toggle_archive(ix, w, cx),
                });
            }),
    )
}

/// Args for `with_chat_ix` — bundled to stay under the arg-count lint.
struct ChatIxArgs<'a> {
    ws: &'a Entity<Workspace>,
    id: u64,
    window: &'a mut Window,
    cx: &'a mut App,
    f: fn(&mut Workspace, usize, &mut Window, &mut Context<Workspace>),
}

/// Resolve `id` to the current index and run `f` — no-op when the chat is
/// gone. Keeps menu closures under the nesting lint.
fn with_chat_ix(a: ChatIxArgs<'_>) {
    a.ws.update(a.cx, |this, cx| {
        if let Some(ix) = this.chat_index(a.id) {
            (a.f)(this, ix, a.window, cx);
        }
    });
}
