use std::rc::Rc;

use gpui_kit::*;

use crate::model::{Chat, MessageKind, Role, ToolStatus};
use crate::workspace::Workspace;

/// How an in-flight rename is driven: the sidebar row's inline editor, or
/// the rename dialog. The row only mounts its editor for `Inline` — a
/// dialog rename shares `Workspace::rename`, so without the split the row
/// would mount an editor whose outside-click commits behind the dialog.
/// Re-exported from `crate::workspace` for its existing callers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenameMode {
    Inline,
    Dialog,
}

impl Workspace {
    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        // Stash the current draft before switching — the composer text
        // belongs to the outgoing chat. `get_mut`: first launch has no chats.
        if let Some(chat) = self.chats.get_mut(self.active) {
            chat.draft = self.composer.read(cx).value().to_string();
        }
        // The outgoing thread keeps its own provider/model/access — the
        // workspace fields are about to be overwritten by the new thread's
        // defaults.
        self.stamp_thread();
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        self.chats.push(Chat::new(id, "New chat"));
        self.active = self.chats.len() - 1;
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        self.find.match_ix = 0;
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
        // Thread defaults: provider+model, access mode, and — for the
        // worktree workspace mode — a fresh git worktree as its cwd. Runs
        // after the scroller reset so a worktree-failure note renders.
        self.apply_thread_defaults(cx);
        let composer = self.composer.clone();
        cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |_this, window, cx| crate::chat_search::focus_new_chat(&composer, window, cx));
        })
        .detach();
        cx.notify();
        self.save();
    }

    pub fn select_chat(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.chats.len() || index == self.active {
            return;
        }
        // Save current draft, restore target's.
        self.chats[self.active].draft = self.composer.read(cx).value().to_string();
        self.stamp_thread();
        self.active = index;
        // The incoming thread's own provider/model/access replace the
        // workspace selection — legacy chats (no stamp) keep it.
        self.restore_thread_selection(cx);
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        self.find.match_ix = 0;
        self.chats[index].unread = false;
        let draft = self.chats[index].draft.clone();
        self.composer.update(cx, |s, cx| {
            s.set_value(draft, window, cx);
            s.focus(window, cx);
        });
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        window.set_window_title(&format!("{} — Rixl Code", self.chats[index].title));
        cx.notify();
        self.save();
        self.save_settings();
    }
}

impl Workspace {
    /// Current vec index of the chat with `id` — positions shift on delete,
    /// so UI closures must capture the id and resolve at action time.
    pub(crate) fn chat_index(&self, id: u64) -> Option<usize> {
        self.chats.iter().position(|c| c.id == id)
    }
}

impl Workspace {
    /// Stop the in-flight reply stream for the active chat.
    pub fn stop_reply(&mut self, cx: &mut Context<Self>) {
        let id = self.chats[self.active].id;
        self.stop_chat_reply(id, cx);
    }

    /// Stop the in-flight reply for chat `id`. Dropping the stream kills
    /// the backend child directly — a hung child would otherwise leak
    /// because the pump thread only ends when the event channel closes.
    pub(crate) fn stop_chat_reply(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        // Snapshot the turn's tool calls onto the agent row while the link
        // still resolves — clearing `run_agent` first would leave the
        // cancelled card with no tool rows.
        self.snapshot_chat_tools(chat_id);
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        drop(chat.stream.take()); // kills the child, sets `cancelled`
        if let Some(task) = chat.reply_task.take() {
            drop(task); // non-detached Task cancels on drop
        }
        chat.running = false;
        chat.complete_turn();
        // A cancelled call never produced a result — close this turn's
        // tool rows so they don't spin forever, and answer any pending
        // approval Deny so the backend's blocked read unblocks.
        let start = chat.messages.iter().rposition(|m| m.role == Role::User).map_or(0, |i| i + 1);
        for msg in Rc::make_mut(&mut chat.messages)[start..].iter_mut() {
            match &mut msg.kind {
                MessageKind::Tool(t) if t.status == ToolStatus::Running => t.status = ToolStatus::Failed,
                MessageKind::Approval(a) if a.decision.is_none() => crate::approval_ops::deny_approval(a),
                _ => {},
            }
        }
        if let Some(id) = chat.run_agent.take()
            && let Some(agent) = self.agents.iter_mut().find(|a| a.id == id)
        {
            agent.status = crate::model::AgentStatus::Cancelled;
            agent.step = "cancelled".into();
            crate::agents::settle_tools(agent);
        }
        self.search_match_ix = 0;
        cx.notify();
        self.save();
    }

    /// Mark the reply finished. `failed_flag` survives so the retry banner
    /// stays visible until the next send/retry clears it.
    pub(crate) fn finish_reply(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        let ok = !self.chats.iter().any(|c| c.id == chat_id && c.failed_flag);
        self.finish_run_agent(chat_id, ok, cx);
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        chat.running = false;
        chat.complete_turn();
        chat.stream = None;
        // The backend stopped waiting — any approval card still showing
        // buttons can no longer reach it, so drop the responder and let
        // the card render as expired.
        for msg in Rc::make_mut(&mut chat.messages).iter_mut() {
            if let MessageKind::Approval(a) = &mut msg.kind {
                a.respond = None;
            }
        }
        if !is_active {
            chat.unread = true;
        }
        self.record_turn_finished(chat_id);
        cx.notify();
        self.save();
    }
}

impl Workspace {
    /// Duplicate chat `ix` (title + messages) as a new chat and select it.
    pub fn duplicate_chat(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.chats.get(ix) else { return };
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        let mut copy = Chat::new(id, format!("{} (copy)", src.title));
        copy.messages = src.messages.clone();
        copy.draft = src.draft.clone();
        self.chats.push(copy);
        let new_ix = self.chats.len() - 1;
        self.select_chat(new_ix, window, cx);
    }

    pub fn toggle_archive(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(ix) {
            chat.archived = !chat.archived;
        }
        // If we archived the active chat, switch to the first non-archived.
        if self.chats[self.active].archived {
            if let Some(next) = self.chats.iter().position(|c| !c.archived) {
                self.select_chat(next, window, cx);
            } else {
                self.new_chat(cx);
            }
        }
        cx.notify();
        self.save();
    }

    /// Run a slash command picked from the composer menu. Routes through
    /// `send` so selection behaves exactly like typing `/cmd` + Enter.
    pub(crate) fn run_command(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |s, cx| {
            s.set_value(format!("/{cmd}"), window, cx);
            s.focus(window, cx);
        });
        self.send(window, cx);
        cx.notify();
    }
}

impl Workspace {
    pub fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(index) {
            chat.pinned = !chat.pinned;
        }
        cx.notify();
        self.save();
    }
}

impl Workspace {
    /// Rough token estimate: chars/4 across the active chat's messages.
    pub fn token_estimate(&self) -> usize {
        self.chats[self.active]
            .messages
            .iter()
            .map(|m| match &m.kind {
                MessageKind::Text(t) => t.len(),
                MessageKind::Tool(t) => t.output.len(),
                MessageKind::Diff(d) => d.hunks.len(),
                MessageKind::Plan(p) => p.steps.iter().map(|s| s.label.len()).sum(),
                MessageKind::Approval(a) => a.detail.len(),
            })
            .sum::<usize>()
            / 4
    }

    pub fn rename_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        self.open_rename(ix, window, cx);
    }

    /// Begin an inline rename on chat `ix` — the sidebar row swaps its title
    /// for `self.rename`, seeded with the current title fully selected.
    pub fn start_inline_rename(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(ix) else { return };
        self.renaming = Some(chat.id);
        self.rename_mode = crate::workspace::RenameMode::Inline;
        self.rename.update(cx, |state, cx| {
            state.set_value(chat.title.clone(), window, cx);
            state.select_all(window, cx);
        });
        // The editor only exists after this render — focus it next frame.
        let input = self.rename.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
        cx.notify();
    }

    /// Abandon the in-flight inline rename without touching the title.
    /// Focus returns to the composer — the hidden input must not keep it.
    pub fn cancel_inline_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            self.composer.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }
}

impl Workspace {
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.save_settings();
        cx.notify();
    }

    pub fn toggle_agents_panel(&mut self, cx: &mut Context<Self>) {
        self.agents_panel_open = !self.agents_panel_open;
        cx.notify();
    }

    /// Toggle the Changes panel; opening refreshes the change list so the
    /// first render never shows stale rows.
    pub fn toggle_changes_panel(&mut self, cx: &mut Context<Self>) {
        self.changes_panel_open = !self.changes_panel_open;
        if self.changes_panel_open {
            self.refresh_changes(cx);
        }
        cx.notify();
    }

    pub fn running_agents(&self) -> usize {
        self.agents.iter().filter(|a| a.status == crate::model::AgentStatus::Running).count()
    }
}

impl Chat {
    /// Record how long the just-finished turn took and clear `started_at`.
    /// Callers: `finish_reply`, `finish_stream` (simulate.rs),
    /// `stop_reply` — each replaces its `chat.started_at = None` with this.
    pub fn complete_turn(&mut self) {
        self.last_turn = self.started_at.take().map(|t| t.elapsed());
    }
}
