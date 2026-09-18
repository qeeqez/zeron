//! Destructive chat operations behind native confirm prompts.

use std::rc::Rc;

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::model::Chat;
use crate::workspace::Workspace;

impl Workspace {
    pub fn delete_chat(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // The last chat can't be deleted — unless it's temporary: closing
        // an ephemeral-only list swaps in a fresh normal chat.
        if index >= self.chats.len() || (self.chats.len() <= 1 && !self.chats[index].ephemeral) {
            return;
        }
        let title = self.chats[index].title.clone();
        let chat = &self.chats[index];
        // A dirty worktree survives the delete — say so up front.
        let detail = if chat.worktree && !crate::worktree::is_clean(std::path::Path::new(&chat.workdir)) {
            "This cannot be undone. Its worktree has uncommitted changes and will be left on disk."
        } else {
            "This cannot be undone."
        };
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            &format!("Delete “{title}”?"),
            Some(detail),
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
        self.selected_chats.remove(&self.chats[index].id);
        // A worktree thread's checkout goes with it — remove before the
        // chat drops so the path is still known. A dirty checkout survives
        // (`remove` refuses it); the feed records the leftover.
        let kept = match crate::worktree::remove_for(self.project.root(), &self.chats[index]) {
            crate::worktree::Removal::Kept(reason) => Some(Self::worktree_kept_entry(&self.chats[index], &reason)),
            crate::worktree::Removal::Removed => None,
        };
        // Chat drop kills the turn: the stream's Drop kills the child.
        self.chats.remove(index);
        if let Some(entry) = kept {
            self.push_activity(entry);
        }
        // Deleting the last (temporary) chat leaves the workspace empty —
        // open a fresh normal chat so there's always something selected.
        if self.chats.is_empty() {
            self.secondary = None;
            self.composer.update(cx, |s, cx| s.set_value("", window, cx));
            self.new_chat(cx);
            crate::dock_badge::update(cx);
            return;
        }
        self.secondary_after_remove(index);
        if self.active >= self.chats.len() {
            self.active = self.chats.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        // The newly-active chat may never have been opened this session.
        self.ensure_messages(self.active);
        self.refresh_pending_bookmarks(cx);
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
            let _ = this.update(cx, |this, cx| this.delete_all_chats_now(cx));
        })
        .detach();
    }

    /// The confirmed wipe shared by `clear_all_chats` and a bulk delete
    /// that selected every chat: worktree checkouts go with their chats
    /// (dirty ones survive and get a feed note), then a fresh chat opens
    /// so the workspace is never empty.
    pub(crate) fn delete_all_chats_now(&mut self, cx: &mut Context<Self>) {
        let kept = crate::worktree::remove_all(self.project.root(), &self.chats);
        self.chats.clear();
        self.note_kept_worktrees(&kept);
        self.search_match_ix = 0;
        self.selected_chats.clear();
        self.secondary = None;
        self.new_chat(cx);
        self.refresh_pending_bookmarks(cx);
        crate::dock_badge::update(cx);
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
        // The dropped tail's first assistant message is the reply being
        // replaced — it joins the new reply's alternatives (newest-first,
        // merged with its own chain) instead of being lost.
        if let Some(outgoing) = chat.messages[ix..].iter().find(|m| m.role == crate::model::Role::Assistant) {
            let mut outgoing = outgoing.clone();
            let mut chain = std::mem::take(&mut outgoing.alternatives);
            let slot = chain.iter().position(|a| a.at < outgoing.at).unwrap_or(chain.len());
            chain.insert(slot, outgoing);
            chat.pending_alternatives = chain;
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

    /// Split the chat with `chat_id` at message `at_ix`: messages `at_ix..`
    /// move into a NEW chat titled "<title> (split)" inserted right after
    /// the source, which keeps `..at_ix`. The split inherits the thread's
    /// provider/model/access/effort/instructions/color/folder stamps and a
    /// worktree thread gets its own checkout (sharing the source's would
    /// break when either chat is deleted). The backend thread id is NOT
    /// copied — the split starts a fresh thread, like `fork_chat`. No-op
    /// at index 0 (nothing would stay behind), past the end, on an empty
    /// chat, or while a reply runs.
    pub fn split_chat(&mut self, chat_id: u64, at_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src_ix) = self.chat_index(chat_id) else { return };
        self.ensure_messages(src_ix);
        let src = &self.chats[src_ix];
        if src.running || at_ix == 0 || at_ix >= src.messages.len() {
            return;
        }
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        // A worktree thread's split needs its own worktree — sharing the
        // source's path unowned would break when either chat is deleted.
        let (workdir, worktree) = if src.worktree {
            match crate::worktree::create(&self.project, id) {
                Ok(dir) => (dir.to_string_lossy().into_owned(), true),
                Err(_) => (self.project.root().to_string_lossy().into_owned(), false),
            }
        } else {
            (src.workdir.clone(), false)
        };
        let mut split = Chat::new(id, format!("{} (split)", src.title));
        let src = &mut self.chats[src_ix];
        split.messages = Rc::new(Rc::make_mut(&mut src.messages).split_off(at_ix));
        split.folder = src.folder.clone();
        split.color = src.color;
        split.title_generated = src.title_generated;
        split.title_custom = src.title_custom;
        split.provider = src.provider.clone();
        split.model = src.model.clone();
        split.access = src.access;
        split.effort = src.effort.clone();
        split.instructions = src.instructions.clone();
        split.workdir = workdir;
        split.worktree = worktree;
        // A temporary chat's split stays temporary — splitting must not
        // silently persist content the user marked ephemeral.
        split.ephemeral = src.ephemeral;
        // Feedback notes are pinned to message timestamps — each follows
        // its message into the split or stays in the source.
        let (moved, kept) = std::mem::take(&mut src.feedback)
            .into_iter()
            .partition(|n| split.messages.iter().any(|m| m.at == n.at));
        split.feedback = moved;
        src.feedback = kept;
        // Checkpoints pinned to moved messages can't resolve in the source
        // (`for_message`'s `at` guard) — prune them like `regenerate_now`.
        // The split gets none: its first turn snapshots fresh.
        src.checkpoints.retain(|c| c.ix < at_ix);
        // The truncated turn earned `last_turn` — don't let the new tail
        // message inherit its duration label.
        src.last_turn = None;
        self.chats.insert(src_ix + 1, split);
        self.select_chat(src_ix + 1, window, cx);
    }

    /// "Split chat…" from the ⋯ menu — a small dialog asking for the
    /// 1-based message number the new chat starts at; OK splits there.
    pub fn open_split_dialog(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_messages_by_id(id);
        let Some(chat) = self.chats.iter().find(|c| c.id == id) else { return };
        if chat.messages.len() < 2 {
            return;
        }
        self.split_input.update(cx, |state, cx| state.set_value("", window, cx));
        let ws = cx.entity();
        let input = self.split_input.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            dialog
                .title("Split chat")
                .overlay_closable(true)
                .child(Input::new(&input).aria_label("Split at message number"))
                .on_ok(move |_, window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_split(id, window, cx));
                    true
                })
        });
        self.split_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Split chat…" — the typed 1-based message number
    /// starts the new chat; anything unparseable or out of range no-ops
    /// inside `split_chat`.
    fn commit_split(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.split_input.read(cx).value().trim().parse::<usize>().unwrap_or(0);
        self.split_chat(id, n.saturating_sub(1), window, cx);
    }

    /// Page message `ix` to an adjacent version of its reply: `older`
    /// steps back through the alternatives, `!older` steps forward to the
    /// newest. The swap keeps the chain's positions stable (see
    /// `ChatMessage::cycle_alternative`).
    pub fn cycle_alternative(&mut self, ix: usize, older: bool, cx: &mut Context<Self>) {
        {
            let chat = &mut self.chats[self.active];
            let Some(msg) = std::rc::Rc::make_mut(&mut chat.messages).get_mut(ix) else { return };
            msg.cycle_alternative(older);
        }
        let pos = self.filtered_pos(ix, cx);
        self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        cx.notify();
        self.save();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "chat_split_tests.rs"]
mod chat_split_tests;

#[cfg(test)]
#[path = "chat_split_ui_tests.rs"]
mod chat_split_ui_tests;

#[cfg(test)]
#[path = "temp_chat_tests.rs"]
mod temp_chat_tests;

#[cfg(test)]
#[path = "regenerate_tests.rs"]
mod regenerate_tests;
