use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

use crate::model::{Agent, Chat};

pub struct Workspace {
    pub chats: Vec<Chat>,
    pub active: usize,
    pub sidebar_collapsed: bool,
    pub agents: Vec<Agent>,
    /// Monotonic id source for agents — survives `clear_finished_agents`.
    pub next_agent_id: u64,
    /// Monotonic id source for chats — survives deletions.
    pub next_chat_id: u64,
    pub agents_panel_open: bool,
    pub sidebar_width: f32,
    pub resizing_sidebar: bool,
    pub composer: Entity<TextareaState>,
    pub search: Entity<InputState>,
    pub scroller: Entity<MessageScrollerState>,
    pub model: SharedString,
    pub mode: SharedString,
    pub palette: Entity<CommandState>,
    pub rename: Entity<InputState>,
    /// Chat id being renamed — stable across deletions, unlike a vec index.
    pub renaming: Option<u64>,
    /// Index into user messages for Cmd+Shift+Up/Down recall cycling.
    pub recall_ix: Option<usize>,
    /// Composer text stashed when a recall cycle starts — restored when the
    /// cycle steps past the newest message.
    pub recall_saved: Option<String>,
    pub chat_search: Entity<InputState>,
    pub chat_search_open: bool,
    pub search_match_ix: usize,
    /// One-shot bypass for the close prompt — `remove_window` re-fires
    /// `on_window_should_close`, so the confirmed path sets this to skip it.
    pub close_confirmed: std::cell::Cell<bool>,
    pub notify_on_done: bool,
    pub word_wrap: bool,
    pub font_size: u8,
    /// Project-relative file paths for the @-mention picker.
    pub project_files: Vec<SharedString>,

    pub backend: std::sync::Arc<dyn crate::backend::AgentBackend>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Ask anything — @ to mention files, / for commands")
        });
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));

        cx.subscribe_in(&search, window, |_this, _s, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe_in(&composer, window, |this, _composer, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
            // `set_value` suppresses Change, so this only fires on real edits.
            InputEvent::Change => {
                this.recall_ix = None;
                this.recall_saved = None;
                cx.notify();
            },
            _ => {},
        })
        .detach();

        let palette = cx.new(|cx| CommandState::new(window, cx));
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Chat title"));
        let chat_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search in chat"));
        cx.subscribe_in(&chat_search, window, |this, _s, event: &InputEvent, _window, cx| match event {
            InputEvent::Change => {
                this.search_match_ix = 0;
                let count = this.filtered_count(cx);
                this.scroller.update(cx, |s, cx| s.reset(count, cx));
                cx.notify();
            },
            InputEvent::PressEnter { shift, .. } => this.jump_to_match(*shift, cx),
            _ => {},
        })
        .detach();
        // Cmd+Q / QuitApp bypasses the window close gate — save drafts here.
        cx.on_app_quit(|this, cx| {
            this.chats[this.active].draft = this.composer.read(cx).value().to_string();
            this.save();
            async {}
        })
        .detach();

        let settings = crate::persist::load_settings();
        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            sidebar_collapsed: settings.sidebar_collapsed,
            sidebar_width: settings.sidebar_width.clamp(180.0, 480.0),
            agents: Vec::new(),
            next_agent_id: 0,
            next_chat_id: 0,
            resizing_sidebar: false,
            agents_panel_open: false,
            composer,
            search,
            scroller,
            model: settings.model.clone().into(),
            mode: settings.mode.clone().into(),
            rename,
            palette,
            renaming: None,
            recall_ix: None,
            recall_saved: None,
            chat_search,
            chat_search_open: false,
            search_match_ix: 0,
            close_confirmed: std::cell::Cell::new(false),
            notify_on_done: settings.notify_on_done,
            word_wrap: settings.word_wrap,
            backend: if settings.use_codex_cli {
                std::sync::Arc::new(crate::backend::CodexCliBackend::new())
            } else {
                std::sync::Arc::new(crate::backend::SimBackend)
            },
            font_size: settings.font_size.clamp(10, 24),
            project_files: Vec::new(),
        };
        let loaded = crate::persist::load_chats(&mut this.next_chat_id);
        if loaded.is_empty() {
            this.new_chat(cx);
        } else {
            this.chats = loaded;
            this.active = settings.active_chat.min(this.chats.len().saturating_sub(1));
        }
        this.start_background(cx);
        this
    }

    pub(crate) fn save(&mut self) {
        // Retention: drop oldest non-pinned chats beyond the cap. Storage
        // order is oldest-first, so retain() hits the oldest first.
        const MAX_CHATS: usize = 50;
        if self.chats.len() > MAX_CHATS {
            let dropped_before_active = self.retention_drops_before_active(MAX_CHATS);
            let mut drop_left = self.chats.len() - MAX_CHATS;
            self.chats.retain(|c| {
                let drop = drop_left > 0 && !c.pinned;
                drop_left -= usize::from(drop);
                !drop
            });
            self.active = self.active.saturating_sub(dropped_before_active).min(self.chats.len().saturating_sub(1));
        }
        crate::persist::save_chats(&self.chats);
    }

    /// How many chats `retain` will drop before `self.active` — the first
    /// `len - max` non-pinned chats go, so count those under the index.
    fn retention_drops_before_active(&self, max: usize) -> usize {
        let mut drop_left = self.chats.len() - max;
        let mut dropped = 0usize;
        for (ix, c) in self.chats.iter().enumerate() {
            if drop_left > 0 && !c.pinned {
                drop_left -= 1;
                dropped += usize::from(ix < self.active);
            }
        }
        dropped
    }

    pub(crate) fn save_settings(&self) {
        // Preserve window bounds saved at close.
        let prev = crate::persist::load_settings();
        crate::persist::save_settings(&crate::persist::Settings {
            model: self.model.to_string(),
            mode: self.mode.to_string(),
            word_wrap: self.word_wrap,
            font_size: self.font_size,
            notify_on_done: self.notify_on_done,
            use_codex_cli: matches!(self.backend.name(), "codex-cli"),
            window_bounds: prev.window_bounds,
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
            active_chat: self.active,
        });
    }

    pub fn toggle_backend(&mut self, cx: &mut Context<Self>) {
        self.backend = if matches!(self.backend.name(), "codex-cli") {
            std::sync::Arc::new(crate::backend::SimBackend)
        } else {
            std::sync::Arc::new(crate::backend::CodexCliBackend::new())
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

    fn delete_chat_now(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Re-check: the prompt is async — chats may have shrunk meanwhile.
        if self.chats.len() <= 1 || index >= self.chats.len() {
            return;
        }
        let was_active = index == self.active;
        // A running chat's child outlives the Chat drop — the pump thread
        // holds the stream's Arc. Kill it explicitly like stop_reply does.
        let chat = &mut self.chats[index];
        if let Some(slot) = chat.child.take() {
            crate::backend::kill_slot(&slot);
        }
        if let Some(task) = chat.reply_task.take() {
            drop(task);
        }
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
}
