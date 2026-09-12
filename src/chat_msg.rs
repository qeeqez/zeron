//! Per-message operations: rate, edit, recall, copy.

use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    pub fn rate_message(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if let Some(msg) = chat.messages.get_mut(ix) {
            msg.rating = if msg.rating == Some(up) { None } else { Some(up) };
        }
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(ix..ix + 1, cx);
        });
        cx.notify();
        self.save();
    }

    /// Load message `ix` into the composer and truncate the chat after it,
    /// so re-sending replaces the original turn.
    pub fn edit_message(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let Some(text) = chat.messages.get(ix).and_then(|m| match &m.kind {
            MessageKind::Text(t) if m.role == Role::User => Some(t.to_string()),
            _ => None,
        }) else {
            return;
        };
        chat.messages.truncate(ix);
        self.recall_ix = None;
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        let count = self.chats[self.active].messages.len();
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
        self.save();
    }

    /// Cmd+Up: load the last user message into the composer (no truncation).
    pub fn recall_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let Some(text) = chat.messages.iter().rev().find_map(|m| match &m.kind {
            MessageKind::Text(t) if m.role == Role::User => Some(t.to_string()),
            _ => None,
        }) else {
            return;
        };
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
    }

    /// Cmd+Shift+Up: cycle backward through user messages (oldest first).
    /// Resets when the composer is edited or a message is sent.
    pub fn recall_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let user_ixs: Vec<usize> = chat
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| (m.role == Role::User && matches!(m.kind, MessageKind::Text(_))).then_some(i))
            .collect();
        if user_ixs.is_empty() {
            return;
        }
        let next = match self.recall_ix {
            Some(i) if i >= user_ixs.len() => 0, // stale index — restart from newest
            Some(i) if i + 1 < user_ixs.len() => i + 1,
            Some(_) => return, // already at the oldest
            None => 0,
        };
        self.recall_ix = Some(next);
        let ix = user_ixs[user_ixs.len() - 1 - next];
        let MessageKind::Text(t) = &chat.messages[ix].kind else { return };
        let text = t.to_string();
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
    }

    /// Cmd+Shift+Down: cycle forward through user messages (newest first).
    /// Past the newest, the composer clears.
    pub fn recall_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cur) = self.recall_ix else {
            // Not cycling — just clear the composer.
            self.composer.update(cx, |s, cx| s.set_value("", window, cx));
            return;
        };
        let chat = &self.chats[self.active];
        let user_ixs: Vec<usize> = chat
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| (m.role == Role::User && matches!(m.kind, MessageKind::Text(_))).then_some(i))
            .collect();
        if cur == 0 {
            self.recall_ix = None;
            self.composer.update(cx, |s, cx| s.set_value("", window, cx));
            return;
        }
        let next = cur - 1;
        self.recall_ix = Some(next);
        // Stale index after truncation — clamp instead of underflowing.
        let Some(&ix) = user_ixs.len().checked_sub(1 + next).and_then(|i| user_ixs.get(i)) else {
            self.recall_ix = None;
            return;
        };
        let MessageKind::Text(t) = &chat.messages[ix].kind else { return };
        let text = t.to_string();
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
    }

    pub fn copy_message(&self, ix: usize, cx: &mut Context<Self>) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let text = match &msg.kind {
            MessageKind::Text(t) => t.to_string(),
            MessageKind::Tool(t) => format!("{}: {}\n{}", t.name, t.detail, t.output),
            MessageKind::Diff(d) => format!("{} (+{} -{})\n{}", d.path, d.added, d.removed, d.hunks),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}
