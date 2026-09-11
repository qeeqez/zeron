use std::time::Duration;

use gpui_kit::component::WindowExt;

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
    pub agents_panel_open: bool,
    pub composer: Entity<TextareaState>,
    pub search: Entity<InputState>,
    pub scroller: Entity<MessageScrollerState>,
    pub model: SharedString,
    pub mode: SharedString,
    pub palette: Entity<CommandState>,
    pub rename: Entity<InputState>,
    pub renaming: Option<usize>,
    pub chat_search: Entity<InputState>,
    pub chat_search_open: bool,
    pub notify_on_done: bool,
    pub word_wrap: bool,
    pub font_size: u8,
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
            InputEvent::Change => cx.notify(),
            _ => {},
        })
        .detach();

        let palette = cx.new(|cx| CommandState::new(window, cx));
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Chat title"));
        let chat_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search in chat"));
        cx.subscribe_in(&chat_search, window, |this, _s, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                let count = this.filtered_count(cx);
                this.scroller.update(cx, |s, cx| s.reset(count, cx));
                cx.notify();
            }
        })
        .detach();
        let settings = crate::persist::load_settings();
        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            sidebar_collapsed: false,
            agents: Vec::new(),
            agents_panel_open: false,
            composer,
            search,
            scroller,
            model: settings.model.clone().into(),
            mode: settings.mode.clone().into(),
            palette,
            rename,
            renaming: None,
            chat_search,
            chat_search_open: false,
            notify_on_done: settings.notify_on_done,
            word_wrap: settings.word_wrap,
            font_size: settings.font_size,
        };
        let loaded = crate::persist::load_chats();
        if loaded.is_empty() {
            this.new_chat(cx);
        } else {
            this.chats = loaded;
        }
        this.start_ticker(cx);
        this
    }

    pub(crate) fn save(&self) {
        crate::persist::save_chats(&self.chats);
        crate::persist::enforce_retention(&self.chats);
    }

    pub(crate) fn save_settings(&self) {
        crate::persist::save_settings(&crate::persist::Settings {
            model: self.model.to_string(),
            mode: self.mode.to_string(),
            word_wrap: self.word_wrap,
            font_size: self.font_size,
            notify_on_done: self.notify_on_done,
        });
    }

    /// Re-render once a second while any chat is running so the elapsed
    /// indicator stays live.
    fn start_ticker(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, Self::tick);
            }
        })
        .detach();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.chats.iter().any(|c| c.running) {
            cx.notify();
        }
    }

    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.chats.push(Chat::new("New chat"));
        self.active = self.chats.len() - 1;
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
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
        self.chats[index].unread = false;
        let draft = self.chats[index].draft.clone();
        self.composer.update(cx, |s, cx| {
            s.set_value(draft, window, cx);
        });
        let count = self.chats[index].messages.len();
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        self.save();
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = cx.entity();
        window.open_sheet(cx, move |sheet, _window, cx| {
            let (notify, font_size) = {
                let s = ws.read(cx);
                (s.notify_on_done, s.font_size)
            };
            sheet.title("Settings").child(crate::views::settings_body(notify, font_size, ws.clone(), cx))
        });
    }

    pub fn open_chat_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_search_open = !self.chat_search_open;
        if self.chat_search_open {
            let input = self.chat_search.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        }
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    }

    pub fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(index) {
            chat.pinned = !chat.pinned;
        }
        cx.notify();
        self.save();
    }

    pub fn delete_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.chats.len() <= 1 || index >= self.chats.len() {
            return;
        }
        self.chats.remove(index);
        if self.active >= self.chats.len() {
            self.active = self.chats.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        let count = self.chats[self.active].messages.len();
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        self.save();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
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
