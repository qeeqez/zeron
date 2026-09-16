//! Open a chat in its own window — the "Open in New Window" item on the
//! chat ⋯ menu and the sidebar row context menu. The chat is shared state
//! (persisted under the project), so the new window loads the same project
//! and selects the chat; the send queue stays per-window, so nothing
//! double-sends.

#[cfg(test)]
mod tests;

use gpui_kit::*;

use crate::workspace::Workspace;

/// How the spawned window finds the chat after loading the project. Chat
/// `id`s are runtime-only — `load_chats` reassigns them in file order, so a
/// deletion shifts every later chat's id. `created_at` is persisted and
/// survives reloads; `index` is the file slot at save time, the fallback
/// for legacy chats whose `created_at` was stamped at load, not stored.
#[derive(Clone, Copy)]
pub(crate) struct LoadedChatKey {
    created_at: std::time::SystemTime,
    index: usize,
}

impl Workspace {
    /// Open chat `id` in a new window bound to this project. The chat file
    /// must exist before the window loads it — `save` first so a chat that
    /// was never persisted (or whose position shifted) is on disk, then
    /// capture the key after the save since eviction can reorder.
    pub fn open_chat_in_new_window(&mut self, id: u64, cx: &mut Context<Self>) {
        // Ephemeral chats never reach disk — a new window couldn't load it.
        if self.chats.iter().any(|c| c.id == id && c.ephemeral) {
            return;
        }
        self.save();
        let Some(index) = self.chat_index(id) else { return };
        let key = LoadedChatKey { created_at: self.chats[index].created_at, index };
        let project = self.project.clone();
        cx.spawn(async move |_, cx| {
            crate::lifecycle::open_chat_window(project, key, cx);
        })
        .detach();
    }

    /// Select the chat a fresh window was opened for. `key` resolves the
    /// chat across the reload (see `LoadedChatKey`); a miss leaves the
    /// persisted active chat selected.
    ///
    /// A freshly loaded window's composer is empty while the active chat
    /// may hold a restored draft — `select_chat` would stash that empty
    /// composer over the draft. Seed it first so the swap preserves both.
    pub(crate) fn select_loaded_chat(&mut self, key: LoadedChatKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self
            .chats
            .iter()
            .position(|c| c.created_at == key.created_at)
            .or(Some(key.index).filter(|ix| *ix < self.chats.len()))
        else {
            return;
        };
        if ix == self.active {
            self.chats[ix].unread = false;
            self.mark_chat_activity_read(self.chats[ix].created_at);
            crate::dock_badge::update(cx);
            cx.notify();
            return;
        }
        let draft = self.chats[self.active].draft.clone();
        self.composer.update(cx, |s, cx| s.set_value(draft, window, cx));
        self.select_chat(ix, window, cx);
    }
}
