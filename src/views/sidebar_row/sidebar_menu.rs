//! The per-chat menu shared by the row's right-click context menu and the
//! hover-revealed "…" dropdown — one builder keeps both in sync.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::workspace::Workspace;

/// Per-row state the menu needs — resolved when the row renders so the
/// labels ("Pin"/"Unpin", "Archive"/"Unarchive") and disabled states are right.
#[derive(Clone, Copy)]
pub(super) struct RowFlags {
    pub pinned: bool,
    pub archived: bool,
    /// The last chat can't be deleted — `delete_chat` no-ops, so the item is
    /// disabled instead of offering a dead action.
    pub only_chat: bool,
}

pub(super) fn chat_row_menu(ws: &Entity<Workspace>, id: u64, flags: RowFlags, menu: PopupMenu) -> PopupMenu {
    let RowFlags { pinned, archived, only_chat } = flags;
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
            f: |this, ix, w, cx| this.start_inline_rename(ix, w, cx),
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
    .item(PopupMenuItem::new("Delete").icon(IconName::Delete).disabled(only_chat).on_click(move |_, w, cx| {
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
