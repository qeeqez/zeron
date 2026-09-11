use std::time::{Duration, SystemTime};

use gpui_kit::component::WindowExt;

use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

use crate::model::{Agent, Chat, ChatMessage, MessageKind, Role};
use crate::simulate::simulate_reply;

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
        cx.subscribe_in(&chat_search, window, |_this, _s, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            sidebar_collapsed: false,
            agents: Vec::new(),
            agents_panel_open: false,
            composer,
            search,
            scroller,
            model: "gpt-5-codex".into(),
            mode: "Agent".into(),
            palette,
            rename,
            renaming: None,
            chat_search,
            chat_search_open: false,
        };
        this.new_chat(cx);
        this.start_ticker(cx);
        this
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
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_sheet(cx, |sheet, _window, _cx| sheet.title("Settings").child(crate::views::settings_body()));
    }

    pub fn open_chat_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_search_open = !self.chat_search_open;
        if self.chat_search_open {
            let input = self.chat_search.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        }
        cx.notify();
    }

    pub fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(index) {
            chat.pinned = !chat.pinned;
        }
        cx.notify();
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

    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats[self.active].running {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let chat = &mut self.chats[self.active];
        if chat.messages.is_empty() {
            chat.title = text.chars().take(40).collect::<String>().into();
        }
        chat.messages.push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(text.into()),
            rating: None,
            at: SystemTime::now(),
        });
        chat.running = true;
        chat.started_at = Some(std::time::Instant::now());
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.scroller.update(cx, |s, cx| {
            s.append(1, cx);
        });
        cx.notify();
        simulate_reply(self, cx);
    }

    /// Re-run the simulated reply for the last assistant message.
    pub fn retry_last(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if chat.running {
            return;
        }
        while matches!(chat.messages.last(), Some(m) if m.role == Role::Assistant) {
            chat.messages.pop();
        }
        chat.running = true;
        chat.started_at = Some(std::time::Instant::now());
        let count = chat.messages.len();
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        simulate_reply(self, cx);
    }
}
