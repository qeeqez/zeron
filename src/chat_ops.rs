use std::rc::Rc;

use gpui_kit::*;

use crate::model::{Chat, MessageKind, Role, ToolStatus};
use crate::workspace::Workspace;
impl Workspace {
    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        // Stash the current draft before switching — the composer text
        // belongs to the outgoing chat. `get_mut`: first launch has no chats.
        if let Some(chat) = self.chats.get_mut(self.active) {
            chat.draft = self.composer.read(cx).value().to_string();
        }
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        self.chats.push(Chat::new(id, "New chat"));
        self.active = self.chats.len() - 1;
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
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
        self.active = index;
        self.recall_ix = None;
        self.recall_saved = None;
        self.search_match_ix = 0;
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

    /// Stop the in-flight reply for chat `id`. Kills the backend child
    /// directly — a hung child would otherwise leak because the pump
    /// thread only drops the stream when it wakes on an event.
    pub(crate) fn stop_chat_reply(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        // Snapshot the turn's tool calls onto the agent row while the link
        // still resolves — clearing `run_agent` first would leave the
        // cancelled card with no tool rows.
        self.snapshot_chat_tools(chat_id);
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        if let Some(slot) = chat.child.take() {
            crate::backend::kill_slot(&slot);
        }
        if let Some(task) = chat.reply_task.take() {
            drop(task); // non-detached Task cancels on drop
        }
        chat.running = false;
        chat.complete_turn();
        // A cancelled call never produced a result — close this turn's
        // tool rows so they don't spin forever.
        let start = chat.messages.iter().rposition(|m| m.role == Role::User).map_or(0, |i| i + 1);
        for msg in Rc::make_mut(&mut chat.messages)[start..].iter_mut() {
            if let MessageKind::Tool(t) = &mut msg.kind
                && t.status == ToolStatus::Running
            {
                t.status = ToolStatus::Failed;
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

    /// Open the native file picker and attach the chosen files.
    pub fn attach_file(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let _ = this.update(cx, |this, cx| this.add_attachments(paths, cx));
        })
        .detach();
    }

    /// Append unique file paths to the active chat's attachments.
    pub(crate) fn add_attachments(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        for name in paths.iter().map(|p| p.to_string_lossy().into_owned()) {
            if !chat.attachments.iter().any(|a| a.as_str() == name) {
                chat.attachments.push(name.into());
            }
        }
        cx.notify();
    }

    pub fn remove_attachment(&mut self, ix: usize, cx: &mut Context<Self>) {
        let attachments = &mut self.chats[self.active].attachments;
        if ix < attachments.len() {
            attachments.remove(ix);
        }
        cx.notify();
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
    /// Cycle sim → codex-cli → http (http only when an endpoint is set).
    pub fn toggle_backend(&mut self, cx: &mut Context<Self>) {
        self.backend = match self.backend.name() {
            "sim" => std::sync::Arc::new(crate::backend::CodexCliBackend::new()),
            "codex-cli" if !self.http_url.is_empty() => {
                std::sync::Arc::new(crate::backend::HttpBackend::new(self.http_url.clone(), self.http_key_env.clone()))
            },
            "codex-cli" => std::sync::Arc::new(crate::backend::SimBackend),
            _ => std::sync::Arc::new(crate::backend::SimBackend),
        };
        self.save_settings();
        cx.notify();
    }

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
    pub fn cancel_inline_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
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

    pub fn running_agents(&self) -> usize {
        self.agents.iter().filter(|a| a.status == crate::model::AgentStatus::Running).count()
    }
}
