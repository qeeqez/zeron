//! Mid-turn steer: inject the composer's text into the running turn.
//!
//! Codex's app-server accepts `turn/steer` on the live connection, so a
//! message sent mid-turn reaches the agent without waiting for the turn to
//! end. Backends without mid-turn input (and a codex turn whose stdin is
//! already gone) fall back to the send queue — the message sends next.

use gpui_kit::*;

use crate::send_queue::Queued;
use crate::workspace::Workspace;

impl Workspace {
    /// Send the composer text as a steer: inject it into the running turn
    /// when the backend supports it, else queue it behind the turn. Slash
    /// commands aren't steerable — they route through `send` unchanged.
    pub fn send_steer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.send_queue.editing_for(self.chats[self.active].id) {
            self.commit_queued_edit(window, cx);
            return;
        }
        if self.send_queue.abandon_edit() {
            self.persist_queue();
        }
        let text = self.composer.read(cx).value().to_string();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.record_prompt(text);
        self.clear_recall();
        // Slash commands run locally or queue — never injected mid-turn.
        // `send_or_queue` also covers the not-running case.
        if crate::slash::is_slash(text) || !self.chats[self.active].running {
            self.send_or_queue(text, window, cx);
            self.clear_composer(window, cx);
            return;
        }
        let prompt = crate::send::build_prompt(text, &self.chats[self.active].attachments);
        if self.chats[self.active].stream.as_ref().is_some_and(|s| s.steer(&prompt)) {
            // Injected — the message joins the turn's transcript now.
            let attachments = std::mem::take(&mut self.chats[self.active].attachments);
            self.push_user_message(Queued::new(text.to_string(), attachments), window, cx);
        } else {
            // No live steer channel (unsupported backend, or the turn's
            // stdin already closed) — queue it to send next.
            self.send_or_queue(text, window, cx);
        }
        self.clear_composer(window, cx);
    }
}
