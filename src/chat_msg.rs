//! Per-message operations: rate, edit, recall, copy, retry — plus the
//! queued-message edit path (a queued item reopens in the composer).

use std::process::{Child, Command};
use std::rc::Rc;

use parking_lot::Mutex;

use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
    pub fn rate_message(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let Some(msg) = Rc::make_mut(&mut chat.messages).get_mut(ix) else { return };
        msg.rating = if msg.rating == Some(up) { None } else { Some(up) };
        let pos = self.filtered_pos(ix, cx);
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(pos..pos + 1, cx);
        });
        cx.notify();
        self.save();
    }

    /// Load message `ix` into the composer and truncate the chat after it,
    /// so re-sending replaces the original turn. Stops any in-flight reply
    /// first — its events would otherwise append to the truncated chat.
    pub fn edit_message(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats[self.active].running {
            self.stop_reply(cx);
        }
        let chat = &mut self.chats[self.active];
        let Some(text) = chat.messages.get(ix).and_then(|m| match &m.kind {
            MessageKind::Text(t) if m.role == Role::User => Some(t.to_string()),
            _ => None,
        }) else {
            return;
        };
        Rc::make_mut(&mut chat.messages).truncate(ix);
        // Truncating drops the turn that earned `last_turn` — don't let the
        // new tail message inherit its duration label.
        chat.last_turn = None;
        self.recall_ix = None;
        self.search_match_ix = 0;
        // Stash the in-progress composer text — recall_next past the newest
        // restores it instead of clearing.
        self.recall_saved = Some(self.composer.read(cx).value().to_string());
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
        self.save();
    }

    /// Cmd+Up: load the last user message into the composer (no truncation).
    /// Seeds the recall cycle so Cmd+Shift+Up continues from here.
    pub fn recall_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let Some(text) = chat.messages.iter().rev().find_map(|m| match &m.kind {
            MessageKind::Text(t) if m.role == Role::User => Some(t.to_string()),
            _ => None,
        }) else {
            return;
        };
        // Stash the in-progress composer text — recall_next past the newest
        // restores it instead of clearing. Kept if a stash already exists
        // (e.g. edit_message saved one).
        if self.recall_saved.is_none() {
            self.recall_saved = Some(self.composer.read(cx).value().to_string());
        }
        self.recall_ix = Some(0);
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
        // Stash the in-progress composer text on first recall — recall_next
        // past the newest restores it instead of clearing.
        if self.recall_saved.is_none() {
            self.recall_saved = Some(self.composer.read(cx).value().to_string());
        }
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
            let saved = self.recall_saved.take().unwrap_or_default();
            self.composer.update(cx, |s, cx| s.set_value(saved, window, cx));
            return;
        }
        let next = cur - 1;
        self.recall_ix = Some(next);
        // Stale index after truncation — clamp instead of underflowing.
        let Some(&ix) = user_ixs.len().checked_sub(1 + next).and_then(|i| user_ixs.get(i)) else {
            self.recall_ix = None;
            self.recall_saved = None;
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
            MessageKind::Plan(p) => p.markdown(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Re-run the reply for the last assistant message.
    pub fn retry_last(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if chat.running {
            return;
        }
        while matches!(chat.messages.last(), Some(m) if m.role == Role::Assistant) {
            Rc::make_mut(&mut chat.messages).pop();
        }
        chat.running = true;
        chat.failed_flag = false;
        chat.started_at = Some(std::time::Instant::now());
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        let (prompt, attachments) = self.chats[self.active]
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| match &m.kind {
                MessageKind::Text(t) => (t.to_string(), m.attachments.clone()),
                _ => (String::new(), vec![]),
            })
            .unwrap_or_default();
        // Re-attach the files — the original prompt included them.
        let prompt = if attachments.is_empty() {
            prompt
        } else {
            let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
            format!("{prompt}\n\n[Attached files: {files}]")
        };
        self.start_reply(&prompt, cx);
    }
}

impl Workspace {
    /// Open a queued message in the composer: the item leaves the queue
    /// (a turn ending mid-edit can't drain stale text) and the composer's
    /// current contents are stashed on the edit for restore-on-commit.
    /// Editing a second row commits the first with the composer's text.
    pub fn edit_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.chats[self.active].id;
        let (draft, chips) = if self.send_queue.editing_for(chat_id) {
            // Commit the open edit first; the new edit inherits its stash —
            // the real draft is what was set aside when editing began, not
            // the item text currently in the composer.
            let text = self.composer.read(cx).value().to_string();
            let attachments = std::mem::take(&mut self.chats[self.active].attachments);
            self.send_queue
                .commit_edit(text, attachments)
                .map(|e| (e.saved_text, e.saved_attachments))
                .unwrap_or_default()
        } else {
            // A parked edit on another chat restores untouched — the
            // composer holds this chat's draft, not that edit's text.
            self.send_queue.abandon_edit();
            (self.composer.read(cx).value().to_string(), std::mem::take(&mut self.chats[self.active].attachments))
        };
        // `draft`/`chips` clone cheap — on a miss they go back.
        let Some(item) = self.send_queue.begin_edit(chat_id, id, draft.clone(), chips.clone()) else {
            // The item drained between render and click — restore the
            // composer as it was (draft text + chips). A chained commit
            // above may still have changed the queue — persist it.
            self.chats[self.active].attachments = chips;
            self.composer.update(cx, |s, cx| s.set_value(draft, window, cx));
            self.persist_queue();
            return;
        };
        self.chats[self.active].attachments = item.attachments.clone();
        self.composer.update(cx, |s, cx| {
            s.set_value(item.text.clone(), window, cx);
            s.focus(window, cx);
        });
        self.persist_queue();
        cx.notify();
    }

    /// Enter while a queued edit is open: write the composer text back at
    /// the item's queue position and restore the stashed draft. An empty
    /// composer cancels — the original message returns untouched.
    pub(crate) fn commit_queued_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        let attachments = std::mem::take(&mut self.chats[self.active].attachments);
        let Some(edit) = self.send_queue.commit_edit(text, attachments) else { return };
        self.chats[self.active].attachments = edit.saved_attachments;
        self.composer.update(cx, |s, cx| {
            s.set_value(edit.saved_text.clone(), window, cx);
            s.focus(window, cx);
        });
        self.persist_queue();
        self.spawn_queue_drain(edit.chat_id, cx);
        cx.notify();
    }
}

/// The single in-flight `say` process — read-aloud is a toggle, so a new
/// click kills whatever is speaking. Finished children are reaped on the
/// next click via `try_wait`.
static SPEECH: Mutex<Option<Child>> = Mutex::new(None);

impl Workspace {
    /// Read message `ix` aloud via macOS `say`; clicking again stops it.
    pub fn speak_message(&self, ix: usize) {
        let Some(msg) = self.chats[self.active].messages.get(ix) else { return };
        let MessageKind::Text(text) = &msg.kind else { return };
        let mut slot = SPEECH.lock();
        if let Some(mut child) = slot.take()
            && child.try_wait().ok().flatten().is_none()
        {
            let _ = child.kill();
            let _ = child.wait(); // reap — kill alone leaves a zombie
            return;
        }
        if let Ok(child) = Command::new("say").arg(&**text).spawn() {
            *slot = Some(child);
        }
    }
}
