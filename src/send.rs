use std::time::SystemTime;

use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

impl Workspace {
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
        self.save();
        self.start_reply(cx);
    }

    /// Re-run the reply for the last assistant message.
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
        self.start_reply(cx);
    }

    /// Dispatch to the real backend or the simulator.
    fn start_reply(&mut self, cx: &mut Context<Self>) {
        if matches!(self.backend.name(), "codex-cli") {
            crate::backend_run::run_backend(self, cx);
        } else {
            crate::simulate::simulate_reply(self, cx);
        }
    }
}
