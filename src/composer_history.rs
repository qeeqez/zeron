//! Composer prompt history — shell-style Up/Down recall over the active
//! chat's `prompt_history` (persisted per chat, see `persist::StoredChat`).
//! Declared from `send.rs` via `#[path]` — `main.rs` is at the SLOC cap.
//!
//! Up starts a recall session when the composer is empty or the cursor sits
//! at document start (offset 0), then steps backward through history;
//! Down steps forward. Leaving the range in either direction restores the
//! pre-recall draft and ends the session. Any real edit (`InputEvent::
//! Change`, `set_value` suppresses it) or send exits recall — see
//! `clear_recall`, which also folds in the older Cmd+Shift+Up message-recall
//! reset so the two cycles can't interleave.

use gpui_kit::*;

use crate::workspace::Workspace;

/// Newest entries win — the oldest fall off once the cap is reached.
const HISTORY_CAP: usize = 100;

/// Append `text` to `history` (newest last): skips empty/whitespace-only
/// prompts and consecutive duplicates, then trims to `HISTORY_CAP`.
fn push_history(history: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if text.is_empty() || history.last().is_some_and(|last| last == text) {
        return;
    }
    history.push(text.to_string());
    if history.len() > HISTORY_CAP {
        let drop = history.len() - HISTORY_CAP;
        history.drain(..drop);
    }
}

impl Workspace {
    /// Record an accepted composer send on the active chat's history.
    /// Called once per send path before the text is consumed.
    pub(crate) fn record_prompt(&mut self, text: &str) {
        push_history(&mut self.chats[self.active].prompt_history, text);
        self.save();
    }

    /// End every composer recall cycle: the Cmd+Shift+Up message recall
    /// (`recall_ix`/`recall_saved`) and the Up/Down history recall
    /// (`history_ix`/`draft_before_recall`). Called on real edits, sends,
    /// chat switches, and anywhere the composer text is replaced.
    pub(crate) fn clear_recall(&mut self) {
        self.recall_ix = None;
        self.recall_saved = None;
        self.history_ix = None;
        self.draft_before_recall.clear();
    }

    /// The draft a live history session would restore, ending the session.
    pub(crate) fn take_history_draft(&mut self) -> Option<String> {
        self.history_ix.take().map(|_| std::mem::take(&mut self.draft_before_recall))
    }

    /// Up/Down inside the composer. Consumes the keystroke (stops
    /// propagation so the textarea's own cursor move never runs) when
    /// history recall applies; propagates when it can't — no history, or
    /// Up with the cursor past document start and no live session.
    pub(crate) fn history_recall(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let consumed = match self.history_ix {
            Some(_) => self.history_step(back, window, cx),
            None if back => self.history_begin(window, cx),
            // Down never starts a session — let the textarea move the cursor.
            None => false,
        };
        if consumed {
            cx.stop_propagation();
        }
    }

    /// First Up of a session: stash the live draft and load the newest
    /// history entry. Only fires from an empty composer or document start,
    /// so mid-text cursor moves keep working.
    fn history_begin(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(newest) = self.chats[self.active].prompt_history.len().checked_sub(1) else { return false };
        let state = self.composer.read(cx);
        if !state.value().trim().is_empty() && state.cursor() != 0 {
            return false;
        }
        // A message-recall stash is the real pre-recall draft — adopt it so
        // stepping past the newest restores what the user actually typed.
        self.draft_before_recall = self.recall_saved.take().unwrap_or_else(|| state.value().to_string());
        self.recall_ix = None;
        self.history_ix = Some(newest);
        let text = self.chats[self.active].prompt_history[newest].clone();
        self.composer.update(cx, |s, cx| s.set_value(text, window, cx));
        cx.notify();
        true
    }

    /// Continue a live session: `back` steps toward the oldest entry,
    /// `!back` toward the newest. Stepping off either end restores the
    /// stashed draft and ends the session.
    fn history_step(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let len = self.chats[self.active].prompt_history.len();
        let ix = self.history_ix.unwrap_or(len);
        // A stale index (history trimmed since the session started) clamps
        // to the newest entry going back, exits going forward.
        let next = if back {
            ix.checked_sub(1).or_else(|| len.checked_sub(1))
        } else {
            Some(ix + 1).filter(|&n| n < len)
        };
        let text = match next {
            Some(n) => self.chats[self.active].prompt_history[n].clone(),
            None => self.take_history_draft().unwrap_or_default(),
        };
        self.history_ix = next;
        self.composer.update(cx, |s, cx| s.set_value(text, window, cx));
        cx.notify();
        true
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "composer_history_tests.rs"]
mod composer_history_tests;
