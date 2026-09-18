//! First-open transcript materialization. `persist::load_chats` leaves
//! `Chat::pending_load` set and `messages` empty; every path that reads or
//! mutates a chat's transcript calls `ensure_messages` first so the lazy
//! load is invisible outside persistence.

use crate::workspace::Workspace;

impl Workspace {
    /// Materialize `chats[ix]`'s transcript if it's still on disk — a
    /// no-op flag check for chats already hydrated.
    pub(crate) fn ensure_messages(&mut self, ix: usize) {
        let Some(chat) = self.chats.get_mut(ix) else { return };
        if chat.pending_load.is_some() && crate::persist::hydrate_chat(chat, &self.project.chats_dir()) {
            // Its stars moved from the badge's pending half to the live
            // half — subtract so the total can't double-count before the
            // next `refresh_pending_bookmarks` lands.
            let starred = chat.messages.iter().filter(|m| m.bookmarked).count();
            self.pending_bookmark_count = self.pending_bookmark_count.saturating_sub(starred);
        }
    }

    /// Same hydration for callers holding a chat id rather than a slot —
    /// reply tasks and background mutations key on ids.
    pub(crate) fn ensure_messages_by_id(&mut self, id: u64) {
        if let Some(ix) = self.chats.iter().position(|c| c.id == id) {
            self.ensure_messages(ix);
        }
    }

    /// Hydrate every pending chat — for panels that aggregate across all
    /// transcripts (bookmarks, usage) where an explicit open is the user's
    /// request for that data.
    pub(crate) fn ensure_all_messages(&mut self) {
        for ix in 0..self.chats.len() {
            self.ensure_messages(ix);
        }
    }
}
