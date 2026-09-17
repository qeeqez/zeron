//! Split view: a second chat rendered read-only beside the active one.
//! `Workspace::secondary` holds its `chats` index — session-only, never
//! persisted. The composer and every chat op stay bound to the active
//! pane; the split pane's titlebar click swaps the panes (via
//! `select_chat`) and its × clears it. Deleting or archiving the split
//! chat clears the pane — the fixups below run from `delete_chat_now`,
//! `toggle_archive`/`archive_selected`, and `save`'s eviction pass.

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Show chat `ix` in the split pane. No-ops on the active chat (it is
    /// already on screen), on the current split chat, and on archived
    /// chats — they stay off screen like the sidebar.
    pub fn open_split(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        if ix == self.active || self.secondary == Some(ix) || chat.archived {
            return;
        }
        self.secondary = Some(ix);
        let count = chat.messages.len();
        self.secondary_scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    }

    /// Close the split pane — the chat itself is untouched.
    pub fn close_split(&mut self, cx: &mut Context<Self>) {
        if self.secondary.take().is_some() {
            cx.notify();
        }
    }

    /// The split pane's titlebar click: make its chat the active one.
    /// `select_chat` swaps the panes — the outgoing active chat takes over
    /// the secondary slot, so both stay on screen.
    pub fn activate_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.secondary {
            self.select_chat(ix, window, cx);
        }
    }

    /// `chats.remove(index)` fixup: the split chat going away clears the
    /// pane; an earlier removal shifts the index down.
    pub(crate) fn secondary_after_remove(&mut self, index: usize) {
        self.secondary = match self.secondary {
            Some(s) if s == index => None,
            Some(s) if s > index => Some(s - 1),
            other => other,
        };
        if self.secondary.is_some_and(|s| s >= self.chats.len()) {
            self.secondary = None;
        }
    }

    /// Clear the pane when its chat fails `keep` — archived chats leave
    /// the sidebar, so they leave the split too.
    pub(crate) fn clear_secondary_if(&mut self, keep: impl Fn(&crate::model::Chat) -> bool) {
        if self.secondary.is_some_and(|s| self.chats.get(s).is_none_or(|c| !keep(c))) {
            self.secondary = None;
        }
    }
}

#[cfg(test)]
#[path = "split_tests.rs"]
mod split_tests;
