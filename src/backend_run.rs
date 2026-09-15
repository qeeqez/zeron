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
    // A signed-out provider can't take a turn — surface the sign-in
    // prompt instead of the backend's opaque auth error.
    if let Some(reason) = this.auth_block_note() {
        this.push_note(format!("**Error:** {reason}"), cx);
        this.finish_reply(chat_id, cx);
        return;
    }
    let model = this.model.to_string();
    let mode = this.mode.to_string();
    // The thread's workdir (project root or its worktree) and access mode
    // travel with the turn — a mid-turn settings change can't alter them.
    let ctx = this.turn_context();
    // Snapshot the workdir before the backend can touch it — the turn's
    // "Undo" restores this checkpoint.
    this.record_turn_checkpoint(chat_id, &ctx.cwd);
    let mut stream = this.backend.send(prompt, &model, &mode, &ctx);
    this.spawn_run_agent(crate::agents::RunAgentSpec { chat_id, name: this.backend.name(), lane: &model }, cx);
    // The pump needs the receiver; the stream itself lands on the chat so
    // stop/delete drop it (killing the child, setting `cancelled`) and the
    // composer can steer into the turn. A dummy receiver stands in — the
    // chat never reads events.
    let (dead_tx, events) = std::sync::mpsc::channel();
    let events = std::mem::replace(&mut stream.events, events);
    drop(dead_tx);
    // The task's future owns this guard: dropping the task (stop, chat
    // delete, quit) drops the future and sets `cancelled` right away —
    // the pump thread's stream drop only fires once it wakes on an event.
    let cancel = stream.cancel_guard();
    let (tx, rx) = std::sync::mpsc::channel::<AgentEvent>();
    std::thread::spawn(move || pump_stream(events, tx));

    let task = cx.spawn(async move |this, cx| {
        let _cancel = cancel;

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
        chat.stream = Some(stream);
        // A fresh turn starts the meter's per-turn counter over.
        chat.usage.begin_turn();
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
            AgentEvent::ApprovalRequest { kind, detail, .. } => {
                self.agent_log(
                    chat_id,
                    crate::agents::AgentLogEntry {
                        line: format!("approval: {} {detail}", kind.label()),
                        count_step: false,
                    },
                    cx,
                );
            },
            _ => {},
        }
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        match ev {
            AgentEvent::TextStart => {
                push_message(chat, MessageKind::Text("".into()));
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
            AgentEvent::TextDelta(text) => self.apply_text_delta(chat_id, &text, cx),
            AgentEvent::ToolCallStart { ix, name, detail } => {
                push_message(
                    chat,
                    MessageKind::Tool(ToolCall {
                        tool_ix: ix,
                        name,
                        detail,
                        output: "".into(),
                        status: ToolStatus::Running,
                        expanded: false,
                    }),
                );
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
            AgentEvent::ToolCallDelta { ix, output } => {
                let pos = update_tool(chat, ix, |t| t.output = format!("{}{}", t.output, output).into());
                if is_active && let Some(pos) = pos {
                    let sp = self.filtered_pos(pos, cx);
                    self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
                }
            },
            AgentEvent::ToolCallSet { ix, output } => {
                let pos = update_tool(chat, ix, |t| t.output = output);
                if is_active && let Some(pos) = pos {
                    let sp = self.filtered_pos(pos, cx);
                    self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
                }
            },
            AgentEvent::ToolCallEnd { ix, ok } => {
                let status = if ok { ToolStatus::Done } else { ToolStatus::Failed };
                let pos = update_tool(chat, ix, |t| t.status = status);
                if is_active && let Some(pos) = pos {
                    let sp = self.filtered_pos(pos, cx);
                    self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
                }
            },
            AgentEvent::ApprovalRequest { ix, kind, detail, respond } => {
                Rc::make_mut(&mut chat.messages).push(ChatMessage {
                    role: Role::Assistant,
                    kind: MessageKind::Approval(crate::backend::ApprovalCard {
                        request_ix: ix,
                        kind,
                        detail,
                        decision: None,
                        respond: Some(respond),
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
            AgentEvent::Diff { path, added, removed, hunks } => {
                push_message(chat, MessageKind::Diff(crate::model::DiffCard { path, added, removed, hunks, expanded: false }));
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
            AgentEvent::Plan { ix, steps } => self.apply_plan(chat_id, ix, steps, cx),
            AgentEvent::Usage { input, output } => {
                // acp's `usage_update` reports context occupancy (used of
                // size), not turn tokens — it feeds the meter's fill, not
                // the counters. Token backends carry input/output.
                let report = if self.backend.name() == "acp" {
                    crate::usage::UsageReport::occupancy(input, output)
                } else {
                    crate::usage::UsageReport::tokens(input, output)
                };
                chat.usage.record(report);
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
                push_message(chat, MessageKind::Text(format!("**Error:** {msg}").into()));
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
        }
        cx.notify();
    }
}

/// Mutate the tool card keyed by `ix`; returns its message index so the
/// caller can re-measure the scroller row when the chat is on screen.
fn update_tool(chat: &mut crate::model::Chat, ix: usize, f: impl FnOnce(&mut ToolCall)) -> Option<usize> {
    let pos = chat.messages.iter().rposition(|m| matches!(&m.kind, MessageKind::Tool(t) if t.tool_ix == ix))?;
    if let MessageKind::Tool(t) = &mut Rc::make_mut(&mut chat.messages)[pos].kind {
        f(t);
    }
    Some(pos)
}

/// Drain the backend event channel into `tx` on a blocking thread.
/// `recv` returns `Err` when the producer exits. The `ReplyStream` itself
/// lives on the chat — dropping it there kills the child.
fn pump_stream(events: std::sync::mpsc::Receiver<AgentEvent>, tx: std::sync::mpsc::Sender<AgentEvent>) {
    while let Ok(e) = events.recv() {
        if tx.send(e).is_err() {
            break;
        }
    }
}

/// Append an assistant message carrying `kind` to the chat.
fn push_message(chat: &mut crate::model::Chat, kind: MessageKind) {
    Rc::make_mut(&mut chat.messages).push(ChatMessage {
        role: Role::Assistant,
        kind,
        rating: None,
        usage: None,
        attachments: vec![],
        at: SystemTime::now(),
    });
}

impl Workspace {
    /// Apply a plan snapshot: replace the card's steps in place, or append
    /// a new plan card on first sight.
    fn apply_plan(&mut self, chat_id: u64, ix: usize, steps: Vec<crate::model::PlanStep>, cx: &mut Context<Self>) {
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let query = if self.chat_search_open {
            self.chat_search.read(cx).value().to_string().to_lowercase()
        } else {
            String::new()
        };
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        if let Some(pos) = chat.messages.iter().rposition(|m| matches!(&m.kind, MessageKind::Plan(p) if p.plan_ix == ix)) {
            if let MessageKind::Plan(p) = &mut Rc::make_mut(&mut chat.messages)[pos].kind {
                p.steps = steps;
            }
            if is_active {
                let sp = self.filtered_pos(pos, cx);
                self.scroller.update(cx, |s, cx| s.remeasure_items(sp..sp + 1, cx));
            }
            return;
        }
        push_message(chat, MessageKind::Plan(crate::model::PlanCard { plan_ix: ix, steps }));
        if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
    }
}
