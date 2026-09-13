use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::*;

use crate::backend::AgentEvent;
use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

/// Drive a real `AgentBackend` reply: spawn the backend, pump its event
/// stream on a thread, and apply events on the UI thread via a channel.
pub fn run_backend(this: &mut Workspace, prompt: &str, cx: &mut Context<Workspace>) {
    let chat_id = this.chats[this.active].id;
    let model = this.model.to_string();
    let mode = this.mode.to_string();
    let stream = this.backend.send(prompt, &model, &mode);
    this.spawn_run_agent(crate::agents::RunAgentSpec { chat_id, name: this.backend.name(), lane: &model }, cx);
    // Share the child slot with the chat so stop/delete can kill a hung
    // process directly — dropping the stream only cancels once the pump
    // thread wakes on the next event.
    let child = stream.child.clone();
    let (tx, rx) = std::sync::mpsc::channel::<AgentEvent>();
    std::thread::spawn(move || pump_stream(stream, tx));

    let task = cx.spawn(async move |this, cx| {
        'outer: loop {
            let e = match rx.try_recv() {
                Ok(e) => e,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    cx.background_executor().timer(Duration::from_millis(30)).await;
                    continue;
                },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            };
            // Only Done ends the turn — item-level errors are non-terminal
            // (codex continues), and every other exit path closes the
            // channel, which surfaces as Disconnected.
            if matches!(e, AgentEvent::Done) {
                break 'outer;
            }
            let _ = this.update(cx, |this, cx| this.apply_event(chat_id, e, cx));
        }
        let _ = this.update_in(cx, |this, window, cx| {
            this.finish_reply(chat_id, cx);
            this.notify_done(chat_id, window, cx);
        });
    });
    if let Some(chat) = this.chats.iter_mut().find(|c| c.id == chat_id) {
        chat.reply_task = Some(task);
        chat.child = child;
    }
}
impl Workspace {
    /// Apply one backend event to the chat identified by `chat_id`.
    /// Chat may have been deleted — events for it are dropped.
    fn apply_event(&mut self, chat_id: u64, ev: AgentEvent, cx: &mut Context<Self>) {
        // Scroller updates only apply to the visible (active) chat, and only
        // when the new message matches an open chat-search query.
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let query = if self.chat_search_open {
            self.chat_search.read(cx).value().to_string().to_lowercase()
        } else {
            String::new()
        };
        // Log to the turn's agent row before borrowing the chat — the row
        // lives on `self.agents`, a disjoint field, but the borrow checker
        match &ev {
            AgentEvent::ToolCallStart { name, detail, .. } => {
                self.agent_log(chat_id, crate::agents::AgentLogEntry { line: format!("{name} {detail}"), count_step: true }, cx);
            },
            AgentEvent::Diff { path, added, removed, .. } => {
                self.agent_log(
                    chat_id,
                    crate::agents::AgentLogEntry {
                        line: format!("diff {path} +{added} -{removed}"),
                        count_step: false,
                    },
                    cx,
                );
            },
            AgentEvent::Error(msg) => {
                self.agent_log(chat_id, crate::agents::AgentLogEntry { line: format!("error: {msg}"), count_step: false }, cx);
            },
            _ => {},
        }
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        match ev {
            AgentEvent::TextStart => {
                Rc::make_mut(&mut chat.messages).push(ChatMessage {
                    role: Role::Assistant,
                    kind: MessageKind::Text("".into()),
                    rating: None,
                    usage: None,
                    attachments: vec![],
                    at: SystemTime::now(),
                });
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
            AgentEvent::TextDelta(text) => self.apply_text_delta(chat_id, &text, cx),
            AgentEvent::ToolCallStart { ix, name, detail } => {
                Rc::make_mut(&mut chat.messages).push(ChatMessage {
                    role: Role::Assistant,
                    kind: MessageKind::Tool(ToolCall {
                        tool_ix: ix,
                        name,
                        detail,
                        output: "".into(),
                        status: ToolStatus::Running,
                        expanded: false,
                    }),
                    rating: None,
                    usage: None,
                    attachments: vec![],
                    at: SystemTime::now(),
                });
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
            AgentEvent::ToolCallDelta { ix, output } => {
                let pos = chat.messages.iter().rposition(|m| matches!(&m.kind, MessageKind::Tool(t) if t.tool_ix == ix));
                if let Some(pos) = pos
                    && let MessageKind::Tool(t) = &mut Rc::make_mut(&mut chat.messages)[pos].kind
                {
                    t.output = format!("{}{}", t.output, output).into();
                }
                if is_active && let Some(pos) = pos {
                    let sp = self.filtered_pos(pos, cx);
                    self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
                }
            },
            AgentEvent::ToolCallEnd { ix, ok } => {
                let status = if ok { ToolStatus::Done } else { ToolStatus::Failed };
                let pos = chat.messages.iter().rposition(|m| matches!(&m.kind, MessageKind::Tool(t) if t.tool_ix == ix));
                if let Some(pos) = pos
                    && let MessageKind::Tool(t) = &mut Rc::make_mut(&mut chat.messages)[pos].kind
                {
                    t.status = status;
                }
                if is_active && let Some(pos) = pos {
                    let sp = self.filtered_pos(pos, cx);
                    self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
                }
            },
            AgentEvent::Diff { path, added, removed, hunks } => {
                Rc::make_mut(&mut chat.messages).push(ChatMessage {
                    role: Role::Assistant,
                    kind: MessageKind::Diff(crate::model::DiffCard { path, added, removed, hunks, expanded: false }),
                    rating: None,
                    usage: None,
                    attachments: vec![],
                    at: SystemTime::now(),
                });
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
            AgentEvent::Usage { input, output } => {
                // Prefer the text reply; fall back to any assistant message.
                let ix = chat
                    .messages
                    .iter()
                    .rposition(|m| m.role == Role::Assistant && matches!(m.kind, MessageKind::Text(_)))
                    .or_else(|| chat.messages.iter().rposition(|m| m.role == Role::Assistant));
                if let Some(ix) = ix {
                    Rc::make_mut(&mut chat.messages)[ix].usage = Some(crate::model::Usage { input, output });
                }
            },
            AgentEvent::Done => {},
            AgentEvent::Error(msg) => {
                chat.failed_flag = true;
                Rc::make_mut(&mut chat.messages).push(ChatMessage {
                    role: Role::Assistant,
                    kind: MessageKind::Text(format!("**Error:** {msg}").into()),
                    rating: None,
                    usage: None,
                    attachments: vec![],
                    at: SystemTime::now(),
                });
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
        }
        cx.notify();
    }
}

impl Workspace {
    /// Append a text delta to the chat's last assistant Text message,
    /// creating the bubble on the first delta.
    fn apply_text_delta(&mut self, chat_id: u64, text: &str, cx: &mut Context<Self>) {
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let query = if self.chat_search_open {
            self.chat_search.read(cx).value().to_string().to_lowercase()
        } else {
            String::new()
        };
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        // Must be an assistant Text message — the last message right after
        // send is the user's own text.
        let needs_new = !matches!(chat.messages.last(), Some(m) if m.role == Role::Assistant && matches!(m.kind, MessageKind::Text(_)));
        if needs_new {
            Rc::make_mut(&mut chat.messages).push(ChatMessage {
                role: Role::Assistant,
                kind: MessageKind::Text("".into()),
                rating: None,
                usage: None,
                attachments: vec![],
                at: SystemTime::now(),
            });
            if is_active && (query.is_empty() || crate::chat_search::msg_matches(chat.messages.last().unwrap(), &query)) {
                self.scroller.update(cx, |s, cx| s.append(1, cx));
            }
        }
        let Some(last) = Rc::make_mut(&mut chat.messages).last_mut() else { return };
        let MessageKind::Text(t) = &mut last.kind else { return };
        *t = format!("{t}{text}").into();
        if is_active {
            let pos = crate::chat_search::last_scroller_pos(&chat.messages, &query);
            self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        }
    }
    /// Mark the reply finished. `failed_flag` survives so the retry banner
    /// stays visible until the next send/retry clears it.
    pub(crate) fn finish_reply(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        let ok = !self.chats.iter().any(|c| c.id == chat_id && c.failed_flag);
        self.finish_run_agent(chat_id, ok, cx);
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        chat.running = false;
        chat.complete_turn();
        chat.child = None;
        if !is_active {
            chat.unread = true;
        }
        cx.notify();
        self.save();
    }
}

/// Drain the backend event channel into `tx` on a blocking thread.
/// `recv` returns `Err` when the producer exits; dropping `stream` here
/// kills the child process if the UI side went away first.
fn pump_stream(stream: crate::backend::ReplyStream, tx: std::sync::mpsc::Sender<AgentEvent>) {
    while let Ok(e) = stream.events.recv() {
        if tx.send(e).is_err() {
            break;
        }
    }
}
