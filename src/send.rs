use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// Slash commands executable locally; anything else falls through to the backend.
pub(crate) const SLASH_COMMANDS: [&str; 6] = ["clear", "compact", "export", "help", "model", "rename"];

/// Split `/cmd arg` into `(cmd, arg)`; `None` when `text` isn't a slash command.
fn slash_cmd(text: &str) -> Option<(&str, &str)> {
    let body = text.strip_prefix('/')?;
    Some(body.split_once(' ').map_or((body, ""), |(c, a)| (c, a.trim())))
}

/// Commands that must run even mid-reply: `/compact` stops the turn itself,
/// `/clear` and `/rename` never touch the message list. Note-producing
/// commands (`/help`, `/model`, `/export`) queue like text instead — run
/// mid-stream their note becomes the last message and the streaming reply
/// appends into (or replaces) it.
fn slash_runs_now(cmd: &str) -> bool {
    matches!(cmd, "clear" | "compact" | "rename")
}

/// Outcome of one queue-drain attempt for a chat.
pub(crate) enum Drain {
    /// A queued item was consumed — a message sent or a command ran.
    Sent,
    /// The chat is active but still busy — check again shortly.
    Wait,
    /// Queue empty, chat deleted, or chat backgrounded — stop draining.
    Done,
}

/// User text plus attachment paths so the backend can open the files.
fn build_prompt(text: &str, attachments: &[SharedString]) -> String {
    if attachments.is_empty() {
        return text.to_string();
    }
    let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
    format!("{text}\n\n[Attached files: {files}]")
}

impl Workspace {
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if self.chats[self.active].running {
            // Codex parity: Enter during a reply queues the message; it sends
            // when the turn ends (see `drain_queued`). Slash commands queue
            // too — except the few that are safe mid-reply (`slash_runs_now`).
            if slash_cmd(text).is_some_and(|(cmd, _)| slash_runs_now(cmd)) && self.run_slash(text, window, cx) {
                self.clear_composer(window, cx);
                return;
            }
            let live: HashSet<u64> = self.chats.iter().map(|c| c.id).collect();
            let chat_id = self.chats[self.active].id;
            // Snapshot the attachments into the queued item — local commands
            // don't consume them, so those stay on the composer.
            let attachments = if slash_cmd(text).is_some_and(|(cmd, _)| SLASH_COMMANDS.contains(&cmd)) {
                Vec::new()
            } else {
                std::mem::take(&mut self.chats[self.active].attachments)
            };
            self.send_queue
                .enqueue(chat_id, Queued::new(text.to_string(), attachments), |id| live.contains(&id));
            self.clear_composer(window, cx);
            self.spawn_queue_drain(chat_id, cx);
            cx.notify();
            return;
        }
        if self.run_slash(text, window, cx) {
            self.clear_composer(window, cx);
            return;
        }
        let attachments = std::mem::take(&mut self.chats[self.active].attachments);
        self.send_text(Queued::new(text.to_string(), attachments), window, cx);
        self.clear_composer(window, cx);
    }

    fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    }

    /// Append `item` as a user message on the active chat and start the
    /// reply. `item.attachments` is the snapshot captured at submit time —
    /// the caller already cleared the live composer list. Caller guarantees
    /// the chat is idle and clears the composer.
    pub(crate) fn send_text(&mut self, item: Queued, window: &mut Window, cx: &mut Context<Self>) {
        let Queued { text, attachments, .. } = item;
        let prompt = build_prompt(&text, &attachments);
        let chat = &mut self.chats[self.active];
        if chat.messages.is_empty() && chat.title == "New chat" {
            let title = text.lines().next().unwrap_or("").chars().take(40).collect::<String>();
            chat.title = title.into();
            window.set_window_title(&format!("{} — Rixl Code", chat.title));
        }
        let display = if attachments.is_empty() {
            text.clone()
        } else {
            let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
            format!("{text}\n\n📎 {files}")
        };
        Rc::make_mut(&mut chat.messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(display.into()),
            rating: None,
            usage: None,
            attachments,
            at: SystemTime::now(),
        });
        chat.running = true;
        chat.failed_flag = false;
        chat.started_at = Some(std::time::Instant::now());
        self.recall_ix = None;
        self.recall_saved = None;

        if self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        cx.notify();
        self.save();
        self.start_reply(&prompt, cx);
    }

    /// Poll the queue on a timer and drain it when the turn ends. Spawned on
    /// enqueue and when a queued chat is re-selected (deduped by `draining_begin`).
    pub(crate) fn spawn_queue_drain(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        if !self.send_queue.draining_begin(chat_id) {
            return;
        }
        cx.spawn(async move |this, cx| {
            while this
                .update_in(cx, |this, window, cx| this.drain_queued(chat_id, window, cx))
                .is_ok_and(|step| !matches!(step, Drain::Done))
            {
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
            let _ = this.update(cx, |this, _cx| this.send_queue.draining_end(chat_id));
        })
        .detach();
    }

    /// Send the next queued message on `chat_id` once its turn ends. Only the
    /// active chat drains (`start_reply` targets `self.active`).
    pub(crate) fn drain_queued(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) -> Drain {
        if self.chat_index(chat_id).is_none() {
            self.send_queue.drop_chat(chat_id);
            return Drain::Done;
        }
        if self.chats.get(self.active).is_none_or(|c| c.id != chat_id) {
            return Drain::Done;
        }
        if self.chats[self.active].running {
            return Drain::Wait;
        }
        let Some(item) = self.send_queue.pop(chat_id) else { return Drain::Done };
        // Queued slash commands run locally now that the stream is over —
        // their notes can no longer corrupt an in-flight reply.
        if !self.run_slash(&item.text, window, cx) {
            self.send_text(item, window, cx);
        }
        Drain::Sent
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

    /// Dispatch to the real backend or the simulator. Only `sim` is fake —
    /// every other backend (codex-cli, http) goes through `run_backend`.
    fn start_reply(&mut self, prompt: &str, cx: &mut Context<Self>) {
        if self.backend.name() == "sim" {
            crate::simulate::simulate_reply(self, cx);
        } else {
            crate::backend_run::run_backend(self, prompt, cx);
        }
    }

    /// Run a `/command` locally. Returns true when the input was consumed.
    fn run_slash(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((cmd, arg)) = slash_cmd(text) else { return false };
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
        // Notes aren't turns — don't let them inherit the last turn's
        // "Worked for Ns" label.
        self.chats[self.active].last_turn = None;
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
