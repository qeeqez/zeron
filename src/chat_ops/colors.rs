//! Chat color tags — a `Chat::color` marks a thread for visual grouping.
//! The sidebar row and the chat titlebar render it as a small dot; the ⋯
//! menu's Color submenu is the picker. `None` clears the tag.

use gpui_kit::*;

use crate::model::ChatColor;
use crate::workspace::Workspace;

impl Workspace {
    /// Tag the chat with `id` with `color`; `None` clears the tag. A no-op
    /// pick skips the redundant save, same as `set_chat_folder`.
    pub fn set_chat_color(&mut self, id: u64, color: Option<ChatColor>, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == id) else { return };
        if chat.color == color {
            return;
        }
        chat.color = color;
        cx.notify();
        self.save();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "../chat_color_tests.rs"]
mod chat_color_tests;
