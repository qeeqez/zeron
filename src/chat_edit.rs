//! Edit-and-resend: a user message can be reopened in an inline editor.
//! Committing forks the conversation — the transcript is truncated at the
//! original message and the edited text resends as a fresh turn, so the
//! old reply and everything after it drops. Cancelling leaves the
//! transcript untouched.

use std::rc::Rc;

use gpui_kit::component::input::TextareaState;
use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// An in-flight message edit: the user message at `ix` on chat `chat_id`
/// is open in `input`. `at` pins the anchor — a message recycled into the
/// same index (after `/clear` or a delete) can't be committed over.
pub struct EditMessage {
    pub chat_id: u64,
    pub ix: usize,
    pub at: std::time::SystemTime,
    pub input: Entity<TextareaState>,
}

impl Workspace {
    /// Open message `ix` in an inline editor. The transcript stays intact
    /// until commit — cancel restores it untouched. Re-clicking Edit on
    /// the message already being edited just refocuses its editor.
    pub fn edit_message(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        if let Some(edit) = self.editing.as_ref().filter(|e| e.chat_id == chat.id && e.ix == ix) {
            let input = edit.input.clone();
            input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        let Some(msg) = chat.messages.get(ix) else { return };
        let MessageKind::Text(text) = &msg.kind else { return };
        if msg.role != Role::User {
            return;
        }
        let (text, at, chat_id) = (strip_attachment_suffix(text, &msg.attachments), msg.at, chat.id);
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8).submit_on_enter(true));
        input.update(cx, |s, cx| s.set_value(text, window, cx));
        // The editor only exists after this render — focus it next frame.
        let focus = input.clone();
        window.defer(cx, move |window, cx| {
            focus.update(cx, |s, cx| s.focus(window, cx));
        });
        self.editing = Some(EditMessage { chat_id, ix, at, input });
        self.remeasure_row(ix, cx);
        cx.notify();
    }

    /// Commit the open edit: drop the original message and everything
    /// after it, then resend the edited text as a fresh turn. The resend
    /// goes through `send_text` → `start_reply`, which checkpoints the
    /// workdir first — the dropped turn's edits stay recoverable via
    /// "Undo turn". Like `/clear`, the truncate doesn't ask.
    pub fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else { return };
        let text = edit.input.read(cx).value().to_string();
        let text = text.trim().to_string();
        let still_there = self.chats.get(self.active).is_some_and(|c| {
            c.id == edit.chat_id
                && matches!(c.messages.get(edit.ix), Some(m) if m.role == Role::User && matches!(m.kind, MessageKind::Text(_)) && m.at == edit.at)
        });
        // An empty commit cancels; a vanished anchor (/clear, delete,
        // another send) abandons the edit — either way nothing is dropped.
        if text.is_empty() || !still_there {
            self.end_edit(edit.ix, window, cx);
            return;
        }
        // Stop an in-flight reply first — its events would otherwise
        // append to the truncated chat.
        if self.chats[self.active].running {
            self.stop_reply(cx);
        }
        let attachments = self.chats[self.active].messages[edit.ix].attachments.clone();
        {
            let chat = &mut self.chats[self.active];
            Rc::make_mut(&mut chat.messages).truncate(edit.ix);
            // Entries pinned to dropped messages are unreachable
            // (`for_message`'s `at` guard) — prune them.
            chat.checkpoints.retain(|c| c.ix < edit.ix);
            // The truncated turn earned `last_turn` — don't let the new
            // tail message inherit its duration label.
            chat.last_turn = None;
        }
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        self.send_text(Queued::new(text, attachments), window, cx);
        self.composer.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Abandon the open edit — the transcript is untouched.
    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else { return };
        self.end_edit(edit.ix, window, cx);
    }

    /// Shared edit teardown: the row re-renders as a bubble and focus
    /// returns to the composer.
    fn end_edit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.remeasure_row(ix, cx);
        self.composer.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Re-measure message `ix`'s scroller row — the bubble↔editor swap
    /// changes its height. No-op when the row is gone or filtered out.
    fn remeasure_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let pos = self.filtered_pos(ix, cx);
        if pos < self.filtered_count(cx) {
            self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        }
    }
}

/// The message's editable text — `push_user_message` appends a "📎 files"
/// line to the stored text; strip it so the editor shows only what the
/// user typed (the attachments resend with the commit).
fn strip_attachment_suffix(text: &SharedString, attachments: &[SharedString]) -> String {
    if attachments.is_empty() {
        return text.to_string();
    }
    let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
    let suffix = format!("\n\n📎 {files}");
    text.strip_suffix(&suffix).unwrap_or(text.as_ref()).to_string()
}
