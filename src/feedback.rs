//! Message feedback: thumbs up/down on assistant replies plus an optional
//! "what went wrong" note. The rating lives on `ChatMessage::rating`
//! (`Some(true)` up, `Some(false)` down, `None` unrated); notes live on the
//! chat's `feedback` list, anchored to the message's `at` timestamp like
//! checkpoints are — a recycled index can't inherit another message's note.
//! Both persist with the chat. There's no feedback endpoint, so this is a
//! local signal rendered on the message.

use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::model::{Chat, Role};
use crate::workspace::Workspace;

/// A "what went wrong" note attached to a thumbs-down. `at` pins the anchor
/// to the message's timestamp — a message recycled into the same index
/// (after `/clear` or a delete) can't inherit it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeedbackNote {
    pub at: std::time::SystemTime,
    pub note: String,
}

/// The message whose note editor is open — `chat_id` + `ix` + `at` pin the
/// anchor so a commit can't land on a different message.
pub struct FeedbackEdit {
    pub chat_id: u64,
    pub ix: usize,
    pub at: std::time::SystemTime,
}

/// Feedback UI state: which message's note editor is open plus the shared
/// input (one editor at a time, like the review comment input).
pub struct FeedbackState {
    pub editing: Option<FeedbackEdit>,
    pub input: Entity<InputState>,
}

impl FeedbackState {
    /// Build the state and wire Enter in the note input to commit.
    /// `subscribe_in` (not `subscribe`) — commit refocuses the composer,
    /// which needs the window.
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("What went wrong? (optional)"));
        cx.subscribe_in(&input, window, |this: &mut Workspace, _input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_feedback(window, cx);
            }
        })
        .detach();
        Self { editing: None, input }
    }
}

impl Workspace {
    /// Set message `ix`'s rating: re-clicking the same thumb clears it,
    /// clicking the other switches. Thumbs-down opens the note editor;
    /// clearing the rating or switching to thumbs-up drops the note.
    pub fn rate_message(&mut self, ix: usize, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let Some(msg) = Rc::make_mut(&mut chat.messages).get_mut(ix) else { return };
        if msg.role != Role::Assistant {
            return;
        }
        msg.rating = if msg.rating == Some(up) { None } else { Some(up) };
        match msg.rating {
            Some(false) => {
                let edit = FeedbackEdit { chat_id: chat.id, ix, at: msg.at };
                self.open_feedback_editor(edit, window, cx);
            },
            _ => {
                // No note without a thumbs-down.
                chat.feedback.retain(|n| n.at != msg.at);
                self.close_feedback_editor(ix, window, cx);
            },
        }
        self.remeasure_feedback_row(ix, cx);
        cx.notify();
        self.save();
    }

    /// Reopen the note editor on a message that already has a note —
    /// the note row under the footer is the affordance.
    pub fn edit_feedback_note(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let Some(msg) = chat.messages.get(ix) else { return };
        if chat.feedback.iter().all(|n| n.at != msg.at) {
            return;
        }
        let edit = FeedbackEdit { chat_id: chat.id, ix, at: msg.at };
        self.open_feedback_editor(edit, window, cx);
        self.remeasure_feedback_row(ix, cx);
        cx.notify();
    }

    /// Commit the note editor: a non-empty value attaches (or replaces) the
    /// note; an empty value removes it — clearing the text is the only
    /// removal path besides toggling the rating off.
    pub fn commit_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.feedback.editing.take() else { return };
        let text = self.feedback.input.read(cx).value().trim().to_string();
        let anchor_alive = {
            let chat = &self.chats[self.active];
            chat.id == edit.chat_id && chat.messages.get(edit.ix).is_some_and(|m| m.at == edit.at)
        };
        if anchor_alive {
            let chat = &mut self.chats[self.active];
            chat.feedback.retain(|n| n.at != edit.at);
            if !text.is_empty() {
                chat.feedback.push(FeedbackNote { at: edit.at, note: text });
            }
        }
        self.end_feedback_edit(edit.ix, window, cx);
        self.save();
    }

    /// Close the note editor without touching the stored note.
    pub fn cancel_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.feedback.editing.take() else { return };
        self.end_feedback_edit(edit.ix, window, cx);
    }

    /// The note attached to the message stamped `at`, if any.
    pub(crate) fn feedback_note(chat: &Chat, at: std::time::SystemTime) -> Option<&str> {
        chat.feedback.iter().find(|n| n.at == at).map(|n| n.note.as_str())
    }

    /// The note editor is open on message `ix` of the active chat.
    pub(crate) fn feedback_editing(&self, ix: usize) -> bool {
        self.feedback
            .editing
            .as_ref()
            .is_some_and(|e| e.chat_id == self.chats[self.active].id && e.ix == ix)
    }

    /// Mount the note editor on message `ix`, seeded with any existing note.
    /// The editor only exists after this render — focus lands next frame.
    fn open_feedback_editor(&mut self, edit: FeedbackEdit, window: &mut Window, cx: &mut Context<Self>) {
        let note = Self::feedback_note(&self.chats[self.active], edit.at).unwrap_or_default().to_string();
        self.feedback.input.update(cx, |s, cx| s.set_value(note, window, cx));
        let input = self.feedback.input.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx));
        });
        self.feedback.editing = Some(edit);
    }

    /// Close the editor if it's open on message `ix` — used when the rating
    /// leaves thumbs-down, where no commit/cancel runs.
    fn close_feedback_editor(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.feedback.editing.as_ref().is_some_and(|e| e.ix == ix) {
            self.feedback.editing = None;
            self.composer.update(cx, |s, cx| s.focus(window, cx));
        }
    }

    /// Shared editor teardown: the row re-renders without the input and
    /// focus returns to the composer.
    fn end_feedback_edit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.remeasure_feedback_row(ix, cx);
        self.composer.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Re-measure message `ix`'s scroller row — the note editor and note
    /// row change its height. No-op when the row is gone or filtered out.
    fn remeasure_feedback_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let pos = self.filtered_pos(ix, cx);
        if pos < self.filtered_count(cx) {
            self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        }
    }
}
