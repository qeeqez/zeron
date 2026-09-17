//! Drag-reorder for sidebar chats. `Chat::order` is the persisted manual
//! position (`0` = unset → `created_at`); `sort_order` is the effective key.
//! A drop on a chat row lands the dragged chat on that row's edge — same
//! group reorders in place, a different folder files it there (the row-drop
//! counterpart of dropping on a folder header). The drop renumbers the
//! target group to dense `base - i` ranks anchored at the group's top key,
//! so every member's file rewrites once and fractional-gap exhaustion can
//! never wedge the order.

use gpui_kit::*;

use crate::model::Chat;
use crate::workspace::Workspace;

/// The effective sidebar sort key: the manual `order` when a drag set one,
/// else `created_at` seconds so untouched chats keep newest-first.
pub(crate) fn sort_order(chat: &Chat) -> i64 {
    if chat.order != 0 {
        return chat.order;
    }
    chat.created_at.duration_since(std::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The full ordering key: `sort_order` alone ties on same-second
/// `created_at`s (two chats created inside one second would sort
/// oldest-first), so sub-second nanos and then `id` break ties
/// newest-first. `order` arithmetic (`drop_chat_on_row`'s `base`) keeps
/// using `sort_order` — the tie-break fields never enter the rank scale.
pub(crate) fn sort_key(chat: &Chat) -> (i64, u32, u64) {
    (sort_order(chat), chat.created_at.duration_since(std::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()), chat.id)
}

/// Sort `ixs` (indices into `ws.chats`) by sidebar position — the within-
/// group order for folders and Unfiled, where recency buckets don't apply.
pub(crate) fn sort_group(ws: &Workspace, ixs: &mut [usize]) {
    ixs.sort_by_key(|ix| std::cmp::Reverse(sort_key(&ws.chats[*ix])));
}

impl Workspace {
    /// Any non-archived chat filed under a folder — the flat recency groups
    /// only render when this is false (mirrors `folder_names`' filter).
    fn has_folders(&self) -> bool {
        self.chats.iter().any(|c| !c.archived && !c.folder.is_empty())
    }

    /// Same sidebar group: with folders the group is the folder name
    /// ("" = Unfiled); without them it's the recency bucket.
    fn same_group(&self, a: usize, b: usize, has_folders: bool) -> bool {
        if has_folders {
            self.chats[a].folder == self.chats[b].folder
        } else {
            self.chat_bucket(a) == self.chat_bucket(b)
        }
    }

    /// A `ChatDrag` may land on `target` when it reorders within the target's
    /// group or — with folders live — files the chat under the target's
    /// folder. Cross-bucket drops in the flat list are meaningless (bucket
    /// membership is derived, not stored), so they're rejected.
    pub(crate) fn can_drop_chat_on(&self, drag: u64, target: u64) -> bool {
        if drag == target {
            return false;
        }
        let (Some(d), Some(t)) = (self.chats.iter().position(|c| c.id == drag), self.chats.iter().position(|c| c.id == target)) else {
            return false;
        };
        if self.chats[t].archived {
            return false;
        }
        let has_folders = self.has_folders();
        has_folders || self.same_group(d, t, has_folders)
    }

    /// Track the hovered drop edge for the indicator line. `place` is
    /// `Some(above)` while a `ChatDrag` hovers `row`'s top/bottom half —
    /// invalid targets and leaving rows pass `None`. Only the row that owns
    /// the current target may clear it, so a non-hovered row's `None` can't
    /// erase a live indicator.
    pub(crate) fn set_chat_drop(&mut self, drag: u64, row: u64, place: Option<bool>, cx: &mut Context<Self>) {
        let new = place
            .filter(|_| self.can_drop_chat_on(drag, row))
            .map(|above| crate::workspace::ChatDrop { row, above });
        if new.is_none() && self.chat_drop.is_some_and(|d| d.row != row) {
            return;
        }
        if self.chat_drop != new {
            self.chat_drop = new;
            cx.notify();
        }
    }

    /// Drop a dragged chat on `target`'s row. Same group reorders it to the
    /// tracked edge; a different folder files it there first, so the row
    /// drop subsumes the header drop's move-to-folder path.
    pub(crate) fn drop_chat_on_row(&mut self, drag: u64, target: u64, cx: &mut Context<Self>) {
        let above = self.chat_drop.take().is_none_or(|d| d.row != target || d.above);
        if !self.can_drop_chat_on(drag, target) {
            return;
        }
        let has_folders = self.has_folders();
        let (Some(d), Some(t)) = (self.chats.iter().position(|c| c.id == drag), self.chats.iter().position(|c| c.id == target)) else {
            return;
        };
        if has_folders && !self.same_group(d, t, has_folders) {
            self.chats[d].folder = self.chats[t].folder.clone();
        }
        // The target group in display order, minus the dragged chat.
        let mut members: Vec<usize> = (0..self.chats.len())
            .filter(|ix| *ix != d && !self.chats[*ix].archived && self.same_group(*ix, t, has_folders))
            .collect();
        sort_group(self, &mut members);
        let insert = members.iter().position(|ix| *ix == t).map_or(members.len(), |p| if above { p } else { p + 1 });
        members.insert(insert, d);
        // Dense ranks anchored at the group's top key — relative order is
        // exact and the scale stays beside `created_at` fallbacks.
        let base = members.iter().map(|ix| sort_order(&self.chats[*ix])).max().unwrap_or(0).max(members.len() as i64);
        for (i, ix) in members.iter().enumerate() {
            self.chats[*ix].order = base - i as i64;
        }
        cx.notify();
        self.save();
    }
}
