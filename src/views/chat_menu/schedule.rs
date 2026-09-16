//! The ⋯ menu's "Schedule…" item — split from `chat_menu.rs` for the SLOC
//! cap (same pattern as `bookmarks.rs`). Opens the schedule dialog for the
//! active chat; disabled on temporary chats since they never persist.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::*;

use crate::workspace::Workspace;

/// "Schedule…" — opens the prompt + interval dialog for the active chat.
/// `ephemeral` disables it: a chat that never reaches disk can't carry a
/// persisted automation.
pub(super) fn schedule_item(ws: &Entity<Workspace>, ephemeral: bool) -> PopupMenuItem {
    let ws = ws.clone();
    PopupMenuItem::new("Schedule…")
        .icon(IconName::CalendarClock)
        .disabled(ephemeral)
        .on_click(move |_, window, cx| {
            ws.update(cx, |this, cx| {
                let id = this.chats[this.active].id;
                this.open_schedule_dialog(id, window, cx);
            });
        })
}
