//! Destructive chat operations behind native confirm prompts.

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub fn delete_chat(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // The last chat can't be deleted — unless it's temporary: closing
        // an ephemeral-only list swaps in a fresh normal chat.
        if index >= self.chats.len() || (self.chats.len() <= 1 && !self.chats[index].ephemeral) {
            return;
        }
        let title = self.chats[index].title.clone();
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            &format!("Delete “{title}”?"),
            Some("This cannot be undone."),
            &[gpui_kit::PromptButton::ok("Delete"), gpui_kit::PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.delete_chat_now(index, window, cx));
        })
        .detach();
    }

    pub(crate) fn delete_chat_now(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Re-check: the prompt is async — chats may have shrunk meanwhile.
        if index >= self.chats.len() || (self.chats.len() <= 1 && !self.chats[index].ephemeral) {
            return;
        }
        let was_active = index == self.active;
        // Deleting the chat mid-rename must end the edit — the row is gone.
        if self.renaming == Some(self.chats[index].id) {
            self.renaming = None;
        }
        // A worktree thread's checkout goes with it — remove before the
        // chat drops so the path is still known.
        crate::worktree::remove_for(self.project.root(), &self.chats[index]);
        // Chat drop kills the turn: the stream's Drop kills the child.
        self.chats.remove(index);
        // Deleting the last (temporary) chat leaves the workspace empty —
        // open a fresh normal chat so there's always something selected.
        if self.chats.is_empty() {
            self.composer.update(cx, |s, cx| s.set_value("", window, cx));
            self.new_chat(cx);
            crate::dock_badge::update(cx);
            return;
        }
        if self.active >= self.chats.len() {
            self.active = self.chats.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        self.clear_recall();
        if was_active {
            // Composer still holds the deleted chat's draft — restore the
            // newly-active chat's draft instead.
            let draft = self.chats[self.active].draft.clone();
            self.composer.update(cx, |s, cx| {
                s.set_value(draft, window, cx);
            });
        }
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        crate::dock_badge::update(cx);
        cx.notify();
        self.save();
    }

    /// Delete every chat and start a fresh one (native confirm).
    pub fn clear_all_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            "Delete all chats?",
            Some("Every conversation will be removed. This cannot be undone."),
            &[gpui_kit::PromptButton::ok("Delete All"), gpui_kit::PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                // Worktree threads' checkouts go with their chats.
                crate::worktree::remove_all(this.project.root(), &this.chats);
                this.chats.clear();
                this.search_match_ix = 0;
                this.renaming = None;
                this.new_chat(cx);
                crate::dock_badge::update(cx);
            });
        })
        .detach();
    }

    /// Regenerate the reply at message `ix`: drop it and everything after,
    /// then re-run the last user prompt. Truncating loses real messages,
    /// so mid-chat regenerates confirm first — regenerating the last
    /// message drops nothing extra and runs straight away.
    pub fn regenerate_from(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.chats[self.active].messages.len();
        if self.chats[self.active].running || ix >= len {
            return;
        }
        // Regenerating the last message drops nothing extra — no confirm.
        if ix + 1 == len {
            self.regenerate_now(ix, cx);
            return;
        }
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            "Regenerate this reply?",
            Some("Regenerating removes this reply and everything after it."),
            &[gpui_kit::PromptButton::ok("Regenerate"), gpui_kit::PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.regenerate_now(ix, cx));
        })
        .detach();
    }

    /// The confirmed regenerate: truncate at `ix` and re-run the last user
    /// prompt. Re-checks `running` — the prompt is async.
    pub(crate) fn regenerate_now(&mut self, ix: usize, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if chat.running || ix >= chat.messages.len() {
            return;
        }
        std::rc::Rc::make_mut(&mut chat.messages).truncate(ix);
        // Entries pinned to dropped messages are unreachable
        // (`for_message`'s `at` guard) — prune them.
        chat.checkpoints.retain(|c| c.ix < ix);
        // The truncated turn earned `last_turn` — don't let the new tail
        // message inherit its duration label.
        chat.last_turn = None;
        self.clear_recall();
        self.rerun_last_prompt(cx);
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "temp_chat_tests.rs"]
mod temp_chat_tests;

#[cfg(test)]
#[path = "regenerate_tests.rs"]
mod regenerate_tests;
