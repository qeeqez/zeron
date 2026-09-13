use std::rc::Rc;
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
        if chat.messages.is_empty() && chat.title == "New chat" {
            let title = text.lines().next().unwrap_or("").chars().take(40).collect::<String>();
            chat.title = title.into();
            window.set_window_title(&format!("{} — Rixl Code", chat.title));
        }
        let display = if chat.attachments.is_empty() {
            text.to_string()
        } else {
            let files = chat.attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
            format!("{text}\n\n📎 {files}")
        };
        Rc::make_mut(&mut chat.messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(display.into()),
            rating: None,
            usage: None,
            attachments: chat.attachments.clone(),
            at: SystemTime::now(),
        });
        chat.running = true;
        chat.failed_flag = false;
        chat.started_at = Some(std::time::Instant::now());
        self.recall_ix = None;
        self.recall_saved = None;

        chat.attachments.clear();
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        if self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
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
            Rc::make_mut(&mut chat.messages).pop();
        }
        chat.running = true;
        chat.failed_flag = false;
        chat.started_at = Some(std::time::Instant::now());
        self.search_match_ix = 0;
        let count = self.filtered_count(cx);
        self.scroller.update(cx, |s, cx| {
            s.reset(count, cx);
        });
        cx.notify();
        let (prompt, attachments) = self.chats[self.active]
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| match &m.kind {
                MessageKind::Text(t) => (t.to_string(), m.attachments.clone()),
                _ => (String::new(), vec![]),
            })
            .unwrap_or_default();
        // Re-attach the files — the original prompt included them.
        let prompt = if attachments.is_empty() {
            prompt
        } else {
            let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
            format!("{prompt}\n\n[Attached files: {files}]")
        };
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
                if self.chats[self.active].running {
                    self.stop_reply(cx);
                }
                let chat = &mut self.chats[self.active];
                let keep = 4.min(chat.messages.len());
                let drain_to = chat.messages.len() - keep;
                Rc::make_mut(&mut chat.messages).drain(..drain_to);
                self.recall_ix = None;
                self.recall_saved = None;
                self.search_match_ix = 0;
                let count = self.filtered_count(cx);
                self.scroller.update(cx, |s, cx| s.reset(count, cx));
                self.save();
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
    pub(crate) fn push_note(&mut self, text: String, cx: &mut Context<Self>) {
        std::rc::Rc::make_mut(&mut self.chats[self.active].messages).push(ChatMessage {
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        });
        if self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        cx.notify();
        self.save();
    }
}
