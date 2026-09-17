//! Message pinning: one message per chat can be pinned — the banner under
//! the titlebar shows its snippet and jumps back to it. The flag rides on
//! the message like `bookmarked`, so a truncated turn drops the pin with
//! the message. Declared from `chat_msg.rs` beside `toggle_bookmark`
//! (`main.rs` and `chat_msg.rs` are at the SLOC cap).

use std::rc::Rc;

use gpui_kit::*;

use crate::workspace::Workspace;

#[cfg(test)]
#[path = "pin_tests.rs"]
mod pin_tests;

impl Workspace {
    /// The chat's pinned message as `(index, &message)` — at most one row
    /// carries the flag (see `toggle_message_pin`).
    pub(crate) fn pinned_message(&self) -> Option<(usize, &crate::model::ChatMessage)> {
        self.chats[self.active].messages.iter().enumerate().find(|(_, m)| m.pinned)
    }

    /// Pin message `ix`, or unpin it when it already carries the flag.
    /// Pinning a second message unpins the first — one pin per chat.
    pub fn toggle_message_pin(&mut self, ix: usize, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let messages = Rc::make_mut(&mut chat.messages);
        let Some(was) = messages.get(ix).map(|m| m.pinned) else { return };
        for m in messages.iter_mut() {
            m.pinned = false;
        }
        messages[ix].pinned = !was;
        cx.notify();
        self.save();
    }

    /// The banner's ×: drop the pin wherever it sits. Clearing every flag
    /// (rather than one index) keeps the one-pin invariant even if a stale
    /// file ever carried two.
    pub fn unpin_message(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let mut cleared = false;
        for m in Rc::make_mut(&mut chat.messages) {
            cleared |= std::mem::take(&mut m.pinned);
        }
        if !cleared {
            return;
        }
        cx.notify();
        self.save();
    }
}
