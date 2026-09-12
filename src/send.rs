use std::time::SystemTime;

use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Slash commands executable locally; anything else falls through to the backend.
const SLASH_COMMANDS: [&str; 6] = ["clear", "compact", "export", "help", "model", "rename"];

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
        if self.run_slash(text, window, cx) {
            self.composer.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            return;
        }
        let prompt = self.build_prompt(text);
        let chat = &mut self.chats[self.active];
        if chat.messages.is_empty() {
            chat.title = text.chars().take(40).collect::<String>().into();
            window.set_window_title(&format!("Rixl Code — {}", chat.title));
        }
        chat.messages.push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(text.into()),
            rating: None,
            usage: None,
            at: SystemTime::now(),
        });
        chat.running = true;
        chat.started_at = Some(std::time::Instant::now());
        chat.attachments.clear();
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.scroller.update(cx, |s, cx| {
            s.append(1, cx);
        });
        cx.notify();
        self.save();
        self.start_reply(&prompt, cx);
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
        let prompt = self.chats[self.active]
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| match &m.kind {
                MessageKind::Text(t) => t.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default();
        self.start_reply(&prompt, cx);
    }

    /// Dispatch to the real backend or the simulator.
    fn start_reply(&mut self, prompt: &str, cx: &mut Context<Self>) {
        if matches!(self.backend.name(), "codex-cli") {
            crate::backend_run::run_backend(self, prompt, cx);
        } else {
            crate::simulate::simulate_reply(self, cx);
        }
    }

    /// User text plus attachment paths so the backend can open the files.
    fn build_prompt(&self, text: &str) -> String {
        let chat = &self.chats[self.active];
        if chat.attachments.is_empty() {
            return text.to_string();
        }
        let files = chat.attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
        format!("{text}\n\n[Attached files: {files}]")
    }

    /// Run a `/command` locally. Returns true when the input was consumed.
    fn run_slash(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(body) = text.strip_prefix('/') else { return false };
        let (cmd, arg) = body.split_once(' ').map_or((body, ""), |(c, a)| (c, a.trim()));
        match cmd {
            "clear" => self.clear_all_chats(window, cx),
            "export" => self.export_active(cx),
            "rename" => self.rename_active(window, cx),
            "model" => {
                if arg.is_empty() {
                    self.push_note(format!("Current model: **{}** — pick one of: {}", self.model, crate::model::MODELS.join(", ")), cx);
                } else if crate::model::MODELS.contains(&arg) {
                    self.model = arg.into();
                    self.save_settings();
                    self.push_note(format!("Model set to **{arg}**"), cx);
                } else {
                    self.push_note(format!("Unknown model `{arg}` — pick one of: {}", crate::model::MODELS.join(", ")), cx);
                }
            },
            "compact" => {
                let chat = &mut self.chats[self.active];
                let keep = 4.min(chat.messages.len());
                chat.messages.drain(..chat.messages.len() - keep);
                self.push_note(format!("Compacted — kept the last {keep} messages."), cx);
            },
            "help" => {
                let list = SLASH_COMMANDS.iter().map(|c| format!("`/{c}`")).collect::<Vec<_>>().join(" ");
                self.push_note(format!("Commands: {list}"), cx);
            },
            _ => return false,
        }
        true
    }

    /// Append a local assistant note (command feedback, not a backend reply).
    fn push_note(&mut self, text: String, cx: &mut Context<Self>) {
        self.chats[self.active].messages.push(ChatMessage {
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            usage: None,
            at: SystemTime::now(),
        });
        self.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
        self.save();
    }
}
