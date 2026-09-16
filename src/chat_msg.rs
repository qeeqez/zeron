//! Per-message operations: recall, retry, quote — plus the queued-message
//! edit path (a queued item reopens in the composer). Copy variants live in
//! `chat_msg::copy`; message rating lives in `crate::feedback`;
//! edit-and-resend of a sent user message lives in `crate::chat_edit`.

pub(crate) mod copy;
#[cfg(test)]
mod copy_tests;

/// The Bookmarks panel — every loaded chat's starred messages in one
/// right-side list. Declared here, not in `main.rs` — the crate root is at
/// the SLOC cap.
pub(crate) mod bookmarks_panel;

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "quote_selection_tests.rs"]
mod quote_selection_tests;

use std::rc::Rc;

use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
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
        // restores it instead of clearing. A live history session's stash is
        // the real draft (the composer holds a recalled entry); otherwise
        // keep an existing stash (e.g. a recall cycle that already saved one).
        if self.recall_saved.is_none() {
            self.recall_saved = Some(self.take_history_draft().unwrap_or_else(|| self.composer.read(cx).value().to_string()));
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
        let user_ixs: Vec<usize> = self.chats[self.active]
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
        // past the newest restores it instead of clearing. A live history
        // session's stash is the real draft, not the recalled entry.
        if self.recall_saved.is_none() {
            self.recall_saved = Some(self.take_history_draft().unwrap_or_else(|| self.composer.read(cx).value().to_string()));
        }
        self.recall_ix = Some(next);
        let ix = user_ixs[user_ixs.len() - 1 - next];
        let MessageKind::Text(t) = &self.chats[self.active].messages[ix].kind else { return };
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
            // Not cycling — just clear the composer. A live history session
            // ends too; its stash is already superseded by the clear.
            self.history_ix = None;
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

    /// Re-run the reply for the last assistant message.
    pub fn retry_last(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if chat.running {
            return;
        }
        let mut popped = Vec::new();
        while matches!(chat.messages.last(), Some(m) if m.role == Role::Assistant) {
            if let Some(m) = Rc::make_mut(&mut chat.messages).pop() {
                popped.push(m);
            }
        }
        // The popped tail's last text message is the reply being replaced —
        // it joins the new reply's alternatives (newest-first, merged with
        // its own chain) instead of being lost.
        let outgoing =
            popped
                .iter()
                .position(|m| matches!(m.kind, MessageKind::Text(_)))
                .or(if popped.is_empty() { None } else { Some(0) });
        if let Some(ix) = outgoing {
            let mut outgoing = popped.swap_remove(ix);
            let mut chain = std::mem::take(&mut outgoing.alternatives);
            let slot = chain.iter().position(|a| a.at < outgoing.at).unwrap_or(chain.len());
            chain.insert(slot, outgoing);
            chat.pending_alternatives = chain;
        }
        self.rerun_last_prompt(cx);
    }

    /// Mark the active chat running and re-send its last user prompt —
    /// shared by `retry_last` and `regenerate_from` once the transcript's
    /// tail is in place.
    pub(crate) fn rerun_last_prompt(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        chat.running = true;
        chat.failed_flag = false;
        chat.started_at = Some(std::time::Instant::now());
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        let prompt = last_user_prompt(&self.chats[self.active]);
        self.start_reply(&prompt, cx);
    }
}

/// The last user message's text with its attachments folded back in —
/// the prompt a retry/regenerate re-sends.
fn last_user_prompt(chat: &crate::model::Chat) -> String {
    let (prompt, attachments) = chat
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
    if attachments.is_empty() {
        prompt
    } else {
        let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
        format!("{prompt}\n\n[Attached files: {files}]")
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
        self.clear_recall();
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
        self.clear_recall();
        self.composer.update(cx, |s, cx| {
            s.set_value(edit.saved_text.clone(), window, cx);
            s.focus(window, cx);
        });
        self.persist_queue();
        self.spawn_queue_drain(edit.chat_id, cx);
        cx.notify();
    }
}

impl Workspace {
    /// Append a text delta to the chat's last assistant Text message,
    /// creating the bubble on the first delta.
    pub(crate) fn apply_text_delta(&mut self, chat_id: u64, text: &str, cx: &mut Context<Self>) {
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let query = if self.chat_search_open {
            self.chat_search.read(cx).value().to_string().to_lowercase()
        } else {
            String::new()
        };
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        // Must be an assistant Text message — the last message right after
        // send is the user's own text.
        let needs_new = !matches!(chat.messages.last(), Some(m) if m.role == Role::Assistant && matches!(m.kind, MessageKind::Text(_)));
        if needs_new {
            let mut msg = crate::model::ChatMessage {
                role: Role::Assistant,
                kind: MessageKind::Text("".into()),
                rating: None,
                bookmarked: false,
                usage: None,
                attachments: vec![],
                at: std::time::SystemTime::now(),
                alternatives: vec![],
            };
            // A regenerate/retry saved the outgoing reply's version chain —
            // this turn's first text bubble inherits it.
            chat.adopt_alternatives(&mut msg);
            Rc::make_mut(&mut chat.messages).push(msg);
            if is_active && (query.is_empty() || crate::chat_search::msg_matches(chat.messages.last().unwrap(), &query)) {
                self.scroller.update(cx, |s, cx| s.append(1, cx));
            }
        }
        let Some(last) = Rc::make_mut(&mut chat.messages).last_mut() else { return };
        let MessageKind::Text(t) = &mut last.kind else { return };
        *t = format!("{t}{text}").into();
        if is_active {
            let pos = crate::chat_search::last_scroller_pos(&chat.messages, &query);
            self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        }
    }
}

impl Workspace {
    /// Star/unstar message `ix` — the chat ⋯ menu's Bookmarks submenu lists
    /// starred rows and jumps back to them. The flag rides on the message,
    /// so a truncated turn drops its bookmarks with it.
    pub fn toggle_bookmark(&mut self, ix: usize, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let Some(msg) = Rc::make_mut(&mut chat.messages).get_mut(ix) else { return };
        msg.bookmarked = !msg.bookmarked;
        cx.notify();
        self.save();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "bookmark_tests.rs"]
mod bookmark_tests;
// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "retry_model_tests.rs"]
mod retry_model_tests;
// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "msg_version_tests.rs"]
mod msg_version_tests;
