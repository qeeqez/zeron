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
    /// Worktree threads get a small glyph in the row suffix (the ⋯ menu
    /// doesn't read this — it's a row-render flag).
    pub worktree: bool,
    /// Temporary chats get a ghost glyph and can't be exported or opened
    /// in a new window — nothing about them reaches disk.
    pub ephemeral: bool,
}

/// What the menu needs to know about its row — the stable chat id plus the
/// per-render flags. Bundled to stay under the arg-count lint.
#[derive(Clone, Copy)]
pub(super) struct RowMenu {
    pub id: u64,
    pub flags: RowFlags,
}

pub(super) fn chat_row_menu(
    ws: &Entity<Workspace>, row: RowMenu, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let RowMenu { id, flags } = row;
    let RowFlags { pinned, archived, only_chat, ephemeral, .. } = flags;
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_dup = ws.clone();
    let ws_export = ws.clone();
    let ws_del = ws.clone();
    let ws_arch = ws.clone();
    let ws_window = ws.clone();
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
    .submenu("Move to folder", window, cx, {
        let ws = ws.clone();
        move |m, _w, cx| folder_submenu(&ws, id, m, cx)
    })
    .item(PopupMenuItem::new("Duplicate").icon(IconName::Copy).on_click(move |_, w, cx| {
        with_chat_ix(ChatIxArgs {
            ws: &ws_dup,
            id,
            window: w,
            cx,
            f: |this, ix, w, cx| this.duplicate_chat(ix, w, cx),
        });
    }))
    .item(
        PopupMenuItem::new("Open in New Window")
            .icon(IconName::WindowRestore)
            .disabled(ephemeral)
            .on_click(move |_, _w, cx| {
                ws_window.update(cx, |this, cx| this.open_chat_in_new_window(id, cx));
            }),
    )
    .item(PopupMenuItem::new("Export").icon(IconName::Share).disabled(ephemeral).on_click(move |_, w, cx| {
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

/// The "Move to folder" submenu: every existing folder (checked when it's
/// the chat's current one), then "New folder…" and — for filed chats —
/// "Remove from folder".
fn folder_submenu(ws: &Entity<Workspace>, id: u64, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let state = ws.read(cx);
    let current = state.chats.iter().find(|c| c.id == id).map(|c| c.folder.clone()).unwrap_or_default();
    let mut menu = state.folder_names().into_iter().fold(menu, |m, name| {
        let ws = ws.clone();
        let folder = name.clone();
        m.item(
            PopupMenuItem::new(name)
                .icon(IconName::Folder)
                .checked(folder == current)
                .on_click(move |_, _w, cx| {
                    ws.update(cx, |this, cx| this.set_chat_folder(id, &folder, cx));
                }),
        )
    });
    if !current.is_empty() {
        let ws = ws.clone();
        menu = menu.item(PopupMenuItem::new("Remove from folder").icon(IconName::FolderOpen).on_click(move |_, _w, cx| {
            ws.update(cx, |this, cx| this.set_chat_folder(id, "", cx));
        }));
    }
    let ws = ws.clone();
    menu.separator()
        .item(PopupMenuItem::new("New folder…").icon(IconName::FolderPlus).on_click(move |_, w, cx| {
            ws.update(cx, |this, cx| this.open_folder_dialog(id, w, cx));
        }))
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
