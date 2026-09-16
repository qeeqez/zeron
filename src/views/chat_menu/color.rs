//! The ⋯ menu's "Color" submenu — split from `chat_menu.rs` for the SLOC
//! cap (same pattern as `chat_menu/bookmarks.rs`).

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use super::color_dot;
use crate::workspace::Workspace;

/// The "Color" submenu: one swatch row per `ChatColor` plus "None" to clear.
/// The current tag reads checked — the swatch carries `aria_toggled` so
/// tests see the same state the check icon shows.
pub(super) fn color_submenu(ws: &Entity<Workspace>, current: Option<crate::model::ChatColor>, menu: PopupMenu) -> PopupMenu {
    use gpui_kit::accesskit::Toggled;
    let menu = crate::model::ChatColor::ALL.into_iter().fold(menu, |m, color| {
        let checked = current == Some(color);
        let ws = ws.clone();
        m.item(
            PopupMenuItem::element(move |_, _| {
                div()
                    .id(format!("color-swatch-{}", color.name()))
                    .test_support()
                    .role(Role::MenuItemRadio)
                    .aria_toggled(if checked { Toggled::True } else { Toggled::False })
                    .aria_label(color.label())
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(color_dot(format!("swatch-dot-{}", color.name()), color, px(10.)))
                    .child(color.label())
            })
            .checked(checked)
            .on_click(move |_, _w, cx| {
                ws.update(cx, |this, cx| {
                    let id = this.chats[this.active].id;
                    this.set_chat_color(id, Some(color), cx);
                });
            }),
        )
    });
    let ws = ws.clone();
    menu.item(PopupMenuItem::new("None").checked(current.is_none()).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| {
            let id = this.chats[this.active].id;
            this.set_chat_color(id, None, cx);
        });
    }))
}
