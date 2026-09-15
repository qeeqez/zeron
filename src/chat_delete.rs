//! Destructive chat operations behind native confirm prompts.

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub fn delete_chat(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats.len() <= 1 || index >= self.chats.len() {
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
        if self.chats.len() <= 1 || index >= self.chats.len() {
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
        if self.active >= self.chats.len() {
            self.active = self.chats.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        self.recall_ix = None;
        self.recall_saved = None;
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
            });
        })
        .detach();
    }
}
