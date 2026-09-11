use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

use crate::model::{Chat, ChatMessage, MessageKind, Role};
use crate::simulate::simulate_reply;

pub struct Workspace {
    pub chats: Vec<Chat>,
    pub active: usize,
    pub sidebar_collapsed: bool,
    pub composer: Entity<TextareaState>,
    pub scroller: Entity<MessageScrollerState>,
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

        cx.subscribe_in(&composer, window, |this, _composer, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(window, cx);
            }
        })
        .detach();

        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            sidebar_collapsed: false,
            composer,
            scroller,
        };
        this.new_chat(cx);
        this
    }

    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.chats.push(Chat::new("New chat"));
        self.active = self.chats.len() - 1;
        self.scroller.update(cx, |s, cx| {
            s.reset(0, cx);
        });
        cx.notify();
    }

    pub fn select_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.chats.len() {
            return;
        }
        self.active = index;
        let count = self.chats[index].messages.len();
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
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
        chat.messages.push(ChatMessage { role: Role::User, kind: MessageKind::Text(text.into()) });
        chat.running = true;
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.scroller.update(cx, |s, cx| {
            s.append(1, cx);
        });
        cx.notify();
        simulate_reply(self, cx);
    }
}
