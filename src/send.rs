use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::send_queue::Queued;
use crate::slash::{is_slash, runs_now};
use crate::workspace::Workspace;

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
pub(crate) fn build_prompt(text: &str, attachments: &[SharedString]) -> String {
    if attachments.is_empty() {
        return text.to_string();
    }
    let files = attachments.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ");
    format!("{text}\n\n[Attached files: {files}]")
}

impl Workspace {
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A queued message parked in the composer commits back into the
        // queue — Enter never starts a new send while an edit is open.
        if self.send_queue.editing_for(self.chats[self.active].id) {
            self.commit_queued_edit(window, cx);
            return;
        }
        // An edit parked on another chat is abandoned — the composer text
        // belongs to this chat's draft now, so the original goes back.
        if self.send_queue.abandon_edit() {
            self.persist_queue();
        }
        let text = self.composer.read(cx).value().to_string();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.send_or_queue(text, window, cx);
        self.clear_composer(window, cx);
    }

    /// Send `text` as the active chat's next message, or queue it behind a
    /// running turn — the shared tail of `send` (composer text) and
    /// `send_review` (diff comments). Slash commands still dispatch locally;
    /// the few safe mid-reply ones run immediately.
    pub(crate) fn send_or_queue(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats[self.active].running {
            // Codex parity: Enter during a reply queues the message; it sends
            // when the turn ends (see `drain_queued`). Slash commands queue
            // too — except the few that are safe mid-reply (`runs_now`).
            if runs_now(text) && self.run_slash(text, window, cx) {
                return;
            }
            let live: HashSet<u64> = self.chats.iter().map(|c| c.id).collect();
            let chat_id = self.chats[self.active].id;
            // Snapshot the attachments into the queued item — local commands
            // don't consume them, so those stay on the composer.
            let attachments = if is_slash(text) { Vec::new() } else { std::mem::take(&mut self.chats[self.active].attachments) };
            self.send_queue
                .enqueue(chat_id, Queued::new(text.to_string(), attachments), |id| live.contains(&id));
            self.persist_queue();
            self.spawn_queue_drain(chat_id, cx);
            cx.notify();
            return;
        }
        if self.run_slash(text, window, cx) {
            return;
        }
        let attachments = std::mem::take(&mut self.chats[self.active].attachments);
        self.send_text(Queued::new(text.to_string(), attachments), window, cx);
    }

    pub(crate) fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    }

    /// Append `item` as a user message on the active chat and start the
    /// reply. `item.attachments` is the snapshot captured at submit time —
    /// the caller already cleared the live composer list. Caller guarantees
    /// the chat is idle and clears the composer.
    pub(crate) fn send_text(&mut self, item: Queued, window: &mut Window, cx: &mut Context<Self>) {
        // A new send abandons a pending message edit — the inline editor
        // unmounts and the transcript stays as it was.
        self.editing = None;
        let prompt = build_prompt(&item.text, &item.attachments);
        self.push_user_message(item, window, cx);
        let chat = &mut self.chats[self.active];
        chat.running = true;
        chat.failed_flag = false;
        chat.started_at = Some(std::time::Instant::now());
        self.recall_ix = None;
        self.recall_saved = None;
        self.start_reply(&prompt, cx);
    }

    /// Append `item` as a user message on the active chat — shared by
    /// `send_text` (new turn) and `send_steer` (mid-turn injection). Sets
    /// the title on a fresh chat and grows the scroller.
    pub(crate) fn push_user_message(&mut self, item: Queued, window: &mut Window, cx: &mut Context<Self>) {
        let Queued { text, attachments, .. } = item;
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
        if self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        cx.notify();
        self.save();
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
        // A pending edit forks the transcript on commit — a queued send
        // landing first would be truncated away, so it waits.
        if self.editing.as_ref().is_some_and(|e| e.chat_id == chat_id) {
            return Drain::Wait;
        }
        let Some(item) = self.send_queue.pop(chat_id) else { return Drain::Done };
        self.persist_queue();
        // Queued slash commands run locally now that the stream is over —
        // their notes can no longer corrupt an in-flight reply.
        if !self.run_slash(&item.text, window, cx) {
            self.send_text(item, window, cx);
        }
        Drain::Sent
    }

    /// Dispatch to the real backend or the simulator. Only `sim` is fake —
    /// every other backend (codex-cli, http) goes through `run_backend`.
    /// A provider with no catalog has no model to send — the turn becomes
    /// an error note instead of a synthetic "default".
    pub(crate) fn start_reply(&mut self, prompt: &str, cx: &mut Context<Self>) {
        if self.model.is_empty() {
            let chat_id = self.chats[self.active].id;
            self.push_note("**Error:** the selected provider has no models — pick a provider with a catalog.".into(), cx);
            self.finish_reply(chat_id, cx);
            return;
        }
        if self.backend.name() == "sim" {
            crate::simulate::simulate_reply(self, cx);
        } else {
            crate::backend_run::run_backend(self, prompt, cx);
        }
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
