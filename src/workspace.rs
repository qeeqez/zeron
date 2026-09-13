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
    /// Agents-panel task input — Enter spawns a standalone backend turn.
    pub task_input: Entity<InputState>,
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
    /// HTTP transport config — kept on the workspace so `toggle_backend`
    /// can rebuild `HttpBackend` without re-reading settings.json.
    pub http_url: String,
    pub http_key_env: String,
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
        let task_input = cx.new(|cx| InputState::new(window, cx).placeholder("New task…"));
        cx.subscribe_in(&task_input, window, |this, s, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let prompt = s.read(cx).value().to_string();
                s.update(cx, |s, cx| s.set_value("", window, cx));
                this.spawn_task_agent(prompt, cx);
            }
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
            model: if crate::model::MODELS.contains(&settings.model.as_str()) {
                settings.model.clone().into()
            } else {
                "default".into()
            },
            task_input,
            mode: if ["Agent", "Plan", "Ask"].contains(&settings.mode.as_str()) {
                settings.mode.clone().into()
            } else {
                "Agent".into()
            },
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
            backend: make_backend(&settings),
            font_size: settings.font_size.clamp(10, 24),
            project_files: Vec::new(),
            http_url: settings.http_url.clone(),
            http_key_env: settings.http_key_env.clone(),
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
            backend: self.backend.name().into(),
            http_url: self.http_url.clone(),
            http_key_env: self.http_key_env.clone(),
            use_codex_cli: None,
            window_bounds: prev.window_bounds,
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
            active_chat: self.active,
        });
    }
}

/// Build the selected backend. `http` falls back to codex-cli when no
/// endpoint is configured — an empty URL would fail every send anyway.
fn make_backend(s: &crate::persist::Settings) -> std::sync::Arc<dyn crate::backend::AgentBackend> {
    match s.backend_name() {
        "sim" => std::sync::Arc::new(crate::backend::SimBackend),
        "http" if !s.http_url.is_empty() => {
            std::sync::Arc::new(crate::backend::HttpBackend::new(s.http_url.clone(), s.http_key_env.clone()))
        },
        _ => std::sync::Arc::new(crate::backend::CodexCliBackend::new()),
    }
}
