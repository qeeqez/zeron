//! Edit-and-resend and undo-turn: a user message can be reopened in an
//! inline editor, or the last turn reverted wholesale. Committing an edit
//! forks the conversation — the transcript is truncated at the original
//! message and the edited text resends as a fresh turn, so the old reply
//! and everything after it drops. Cancelling leaves the transcript
//! untouched. "Undo turn" instead restores the workdir to the turn's
//! checkpoint, drops the turn, and seeds the composer with the message.

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
            // An edit-resend starts a different turn — a version chain
            // parked by an earlier regenerate belongs to the old prompt.
            chat.pending_alternatives.clear();
            // The truncated turn earned `last_turn` — don't let the new
            // tail message inherit its duration label.
            chat.last_turn = None;
        }
        self.clear_recall();
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

    /// "Undo turn" on the last user message: restore the workdir to the
    /// checkpoint taken before its turn, drop the turn's messages, and
    /// seed the composer with the message text so it can be rephrased and
    /// resent. A turn whose restore touches files confirms first, naming
    /// the count; a clean diff undoes on one click. No-op while a reply
    /// runs or the message has no checkpoint (e.g. a loaded old chat).
    pub fn undo_turn(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        if chat.running || chat.messages.iter().rposition(|m| m.role == Role::User) != Some(ix) {
            return;
        }
        let Some(turn) = crate::checkpoints::for_message(chat, ix) else { return };
        if !matches!(chat.messages[ix].kind, MessageKind::Text(_)) {
            return;
        }
        let at = turn.at;
        let workdir = crate::worktree::workdir_for(chat, self.project.root());
        let files = crate::snapshot_store::describe_files(&workdir, &turn.checkpoint).1;
        if files.as_ref().is_some_and(|f| f.is_empty()) {
            self.run_undo_turn(ix, at, window, cx);
            return;
        }
        let title = match &files {
            Some(files) => format!("Undo turn and restore {} {}?", files.len(), if files.len() == 1 { "file" } else { "files" }),
            None => "Undo last turn?".to_string(),
        };
        let detail = files.as_ref().map(|files| {
            let mut names: Vec<&str> = files.iter().take(5).map(|f| f.path.as_str()).collect();
            if files.len() > 5 {
                names.push("…");
            }
            names.join(", ")
        });
        let rx = window.prompt(
            PromptLevel::Warning,
            &title,
            detail.as_deref(),
            &[PromptButton::ok("Undo turn"), PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.run_undo_turn(ix, at, window, cx));
        })
        .detach();
    }

    /// The undo itself, once any confirm is answered: restore the workdir,
    /// then truncate the transcript at `ix` and seed the composer. The
    /// anchor is re-checked — a send or /clear while the confirm was open
    /// leaves everything alone.
    fn run_undo_turn(&mut self, ix: usize, at: std::time::SystemTime, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        if chat.running || chat.messages.iter().rposition(|m| m.role == Role::User) != Some(ix) {
            return;
        }
        let Some(msg) = chat.messages.get(ix) else { return };
        if msg.at != at {
            return;
        }
        let MessageKind::Text(raw) = &msg.kind else { return };
        let text = strip_attachment_suffix(raw, &msg.attachments);
        let attachments = msg.attachments.clone();
        let workdir = crate::worktree::workdir_for(chat, self.project.root());
        let Some(turn) = crate::checkpoints::for_message(chat, ix) else { return };
        let checkpoint = turn.checkpoint.clone();
        if let Err(e) = crate::checkpoints::restore(&workdir, &checkpoint) {
            self.push_note(format!("**Undo failed:** {e}"), cx);
            return;
        }
        // A queued message parked in the composer goes back to the queue —
        // the undone prompt takes the composer over.
        if self.send_queue.abandon_edit() {
            self.persist_queue();
        }
        {
            let chat = &mut self.chats[self.active];
            Rc::make_mut(&mut chat.messages).truncate(ix);
            // Entries pinned to dropped messages are unreachable
            // (`for_message`'s `at` guard) — prune them.
            chat.checkpoints.retain(|c| c.ix < ix);
            // An undo starts a different turn — a version chain parked by
            // an earlier regenerate belongs to the old prompt.
            chat.pending_alternatives.clear();
            // The truncated turn earned `last_turn` — don't let the new
            // tail message inherit its duration label.
            chat.last_turn = None;
            chat.attachments = attachments;
        }
        // Stash the draft the seed overwrites — Cmd+Shift+Down past the
        // newest restores it, same as after `recall_last`.
        if self.recall_saved.is_none() {
            self.recall_saved = Some(self.take_history_draft().unwrap_or_else(|| self.composer.read(cx).value().to_string()));
        }
        self.recall_ix = Some(0);
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        self.composer.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        if self.changes_panel_open {
            self.refresh_changes(cx);
        }
        if self.snapshots.open {
            self.refresh_snapshots(cx);
        }
        self.save();
        cx.notify();
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
pub(crate) fn strip_attachment_suffix(text: &SharedString, attachments: &[SharedString]) -> String {
    if attachments.is_empty() {
        return text.to_string();
    }
    let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
    let suffix = format!("\n\n📎 {files}");
    text.strip_suffix(&suffix).unwrap_or(text.as_ref()).to_string()
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "undo_turn_tests.rs"]
mod undo_turn_tests;
