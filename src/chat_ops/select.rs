//! Sidebar multi-selection: Cmd-click toggles chats into `selected_chats`,
//! and the bar at the sidebar's foot applies Archive/Delete to the set.
//! Selection is by chat id — positions shift as rows are deleted.

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Cmd-click on a chat row: toggle its id in the selection set.
    /// Unknown ids (a row's chat was deleted mid-render) no-op.
    pub fn toggle_chat_selection(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        if self.chat_index(chat_id).is_none() {
            return;
        }
        if !self.selected_chats.remove(&chat_id) {
            self.selected_chats.insert(chat_id);
        }
        cx.notify();
    }

    /// Drop the multi-selection — Esc, a plain row click, the bar's Clear.
    pub fn clear_chat_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected_chats.is_empty() {
            return;
        }
        self.selected_chats.clear();
        cx.notify();
    }

    /// Enter on the focused sidebar (the "sidebar" key context — a Cmd-click
    /// lands focus there): rename the single selected row inline, like
    /// Finder's Enter. Anything else propagates — multi-select has no single
    /// rename target, and a focused descendant (a button, the search field)
    /// owns its own Enter. The row must actually render: a selected chat
    /// filtered out of the list would arm a rename with no editor to catch
    /// it, so the gate mirrors `render_sidebar`'s title+chip predicate.
    pub fn rename_selected_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar_focus.is_focused(window)
            || self.renaming.is_some()
            || self.settings_open
            || self.sidebar_tab != crate::views::sidebar::SidebarTab::Chats
        {
            cx.propagate();
            return;
        }
        let query = self.search.read(cx).value().to_lowercase();
        let [ix] = *self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                self.selected_chats.contains(&c.id)
                    && (query.is_empty() || c.title.to_lowercase().contains(&query))
                    && self.sidebar_filters.matches(c)
            })
            .map(|(ix, _)| ix)
            .collect::<Vec<_>>()
        else {
            cx.propagate();
            return;
        };
        self.start_inline_rename(ix, window, cx);
    }

    /// Selected ids that still resolve to a chat — the set can hold stale
    /// ids between a delete and the next render.
    fn selected_ids(&self) -> Vec<u64> {
        self.chats.iter().filter(|c| self.selected_chats.contains(&c.id)).map(|c| c.id).collect()
    }

    /// Archive every selected chat — the same end state as the row menu's
    /// Archive (`toggle_archive`), applied to the set: `archived` goes on,
    /// the selection drops, and if the active chat got archived the
    /// selection moves to the first live chat (or a fresh one).
    pub fn archive_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        for chat in &mut self.chats {
            if self.selected_chats.contains(&chat.id) {
                chat.archived = true;
            }
        }
        self.selected_chats.clear();
        if self.chats[self.active].archived {
            if let Some(next) = self.chats.iter().position(|c| !c.archived) {
                self.select_chat(next, window, cx);
            } else {
                self.new_chat(cx);
            }
        }
        // Archiving the split-pane chat clears the pane — including when
        // the active fixup just swapped it in.
        self.clear_secondary_if(|c| !c.archived);
        cx.notify();
        self.save();
    }

    /// Delete every selected chat behind one confirm. Chats with a reply
    /// in flight are skipped — deleting a running chat would kill the
    /// turn mid-stream, so the confirm's detail names the skip and a
    /// toast reports it after the delete lands.
    pub fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (running, targets): (Vec<u64>, Vec<u64>) =
            self.selected_ids().into_iter().partition(|id| self.chats.iter().any(|c| c.id == *id && c.running));
        if targets.is_empty() {
            if !running.is_empty() {
                window.push_notification(Notification::warning("Selected chats are still running — nothing to delete"), cx);
            }
            return;
        }
        let n = targets.len();
        let detail = if running.is_empty() {
            "This cannot be undone.".to_string()
        } else {
            format!("This cannot be undone. {} running chat(s) will be skipped.", running.len())
        };
        let rx = window.prompt(
            gpui_kit::PromptLevel::Warning,
            &format!("Delete {n} chat(s)?"),
            Some(detail.as_str()),
            &[gpui_kit::PromptButton::ok("Delete"), gpui_kit::PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.delete_selected_now(targets, running.len(), window, cx));
        })
        .detach();
    }

    /// The confirmed bulk delete: resolve each id fresh (the prompt was
    /// async — positions shifted), then reuse the single-chat removal.
    /// Selecting every chat takes the clear-all path so the workspace
    /// never ends up empty.
    fn delete_selected_now(&mut self, ids: Vec<u64>, skipped: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ids.len() == self.chats.len() {
            self.delete_all_chats_now(cx);
        } else {
            self.delete_chats_by_id(&ids, window, cx);
        }
        self.selected_chats.clear();
        if skipped > 0 {
            window.push_notification(Notification::warning(format!("Skipped {skipped} running chat(s) — stop the reply to delete")), cx);
        }
        cx.notify();
    }

    /// Delete each listed chat — ids resolve fresh inside the loop because
    /// every removal shifts the positions of the chats still queued.
    fn delete_chats_by_id(&mut self, ids: &[u64], window: &mut Window, cx: &mut Context<Self>) {
        for id in ids {
            if let Some(ix) = self.chat_index(*id) {
                self.delete_chat_now(ix, window, cx);
            }
        }
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "select_tests.rs"]
mod select_tests;

#[cfg(test)]
#[path = "rename_shortcut_tests.rs"]
mod rename_shortcut_tests;
