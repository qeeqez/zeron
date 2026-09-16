//! The ⋯ menu's "Bookmarks" submenu — split from `chat_menu.rs` for the
//! SLOC cap (same pattern as `sidebar_row/sidebar_menu.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::workspace::Workspace;

/// The "Bookmarks" submenu: one row per starred message, numbered, labeled
/// with the first ~60 chars of its text. Clicking scrolls the transcript to
/// the message via `scroll_to_message`; an empty list shows a disabled
/// "No bookmarks" row.
pub(super) fn bookmarks_submenu(menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let this = ws.read(cx);
    let bookmarks: Vec<(usize, String)> = this.chats[this.active]
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.bookmarked)
        .map(|(ix, m)| (ix, crate::chat_msg::bookmarks_panel::snippet(m, 60)))
        .collect();
    let ws = ws.clone();
    menu.submenu_with_icon(Some(IconName::Star.into()), "Bookmarks", window, cx, move |m, _w, _cx| {
        if bookmarks.is_empty() {
            return m.item(PopupMenuItem::new("No bookmarks").disabled(true));
        }
        bookmarks.iter().enumerate().fold(m, |m, (n, (ix, label))| {
            let ws = ws.clone();
            let ix = *ix;
            let label = format!("{}. {label}", n + 1);
            m.item(PopupMenuItem::new(label).on_click(move |_, _w, cx| {
                ws.update(cx, |this, cx| this.scroll_to_message(ix, cx));
            }))
        })
    })
}
