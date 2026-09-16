//! The ⋯ menu's "Continue with" submenu — split from `chat_menu.rs` for
//! the SLOC cap (same pattern as `chat_menu/bookmarks.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::workspace::Workspace;

/// The "Continue with" submenu: one row per enabled provider instance other
/// than the chat's own — a pick forks the transcript onto that provider via
/// `continue_chat_with` (fresh backend thread, new chat selected). Disabled
/// while a reply is running, on an empty transcript, or when no other
/// provider is enabled — a disabled submenu still expands on hover, so the
/// off state renders as a plain disabled row instead.
pub(super) fn continue_with_submenu(
    menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let this = ws.read(cx);
    let chat = &this.chats[this.active];
    // Legacy chats (empty stamp) ride the live selection — same rule
    // `continue_chat_with` applies.
    let current = if chat.provider.is_empty() { this.selected_provider.as_str() } else { chat.provider.as_str() };
    let targets: Vec<(String, String, IconName)> = this
        .enabled_providers()
        .into_iter()
        .filter(|p| p.id != current)
        .map(|p| (p.id.clone(), p.name.clone(), p.kind.info().icon))
        .collect();
    let off = chat.running || chat.messages.is_empty() || targets.is_empty();
    if off {
        return menu.item(PopupMenuItem::new("Continue with").icon(IconName::ArrowRightLeft).disabled(true));
    }
    let ws_sub = ws.clone();
    let submenu = PopupMenu::build(window, cx, move |m, _w, _cx| {
        targets.iter().fold(m, |m, (pid, name, icon)| m.item(continue_item(&ws_sub, pid, name, *icon)))
    });
    menu.item(PopupMenuItem::submenu("Continue with", submenu).icon(IconName::ArrowRightLeft))
}

/// One submenu row: the instance's name + kind icon; a pick hands the
/// active chat off to that provider.
fn continue_item(ws: &Entity<Workspace>, pid: &str, name: &str, icon: IconName) -> PopupMenuItem {
    let ws = ws.clone();
    let pid = pid.to_string();
    PopupMenuItem::new(name.to_string()).icon(icon).on_click(move |_, window, cx| {
        ws.update(cx, |this, cx| this.continue_chat_with(this.active, &pid, window, cx));
    })
}
