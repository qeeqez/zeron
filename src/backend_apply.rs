//! Apply streamed `AgentEvent`s to a chat — split from `backend_run.rs`
//! so both stay under the SLOC cap. `run_backend` owns the pump/task
//! lifecycle; everything here is per-event mutation on the UI thread.

use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::*;

use crate::backend::AgentEvent;
use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

impl Workspace {
    /// Apply a batch of backend events to the chat identified by
    /// `chat_id` with one render notify for the lot — a fast backend
    /// bursts deltas far quicker than one render each can keep up with.
    /// Chat may have been deleted — events for it are dropped.
    pub(crate) fn apply_events(&mut self, chat_id: u64, evs: Vec<AgentEvent>, cx: &mut Context<Self>) {
        for ev in evs {
            self.apply_one(chat_id, ev, cx);
        }
        cx.notify();
    }

    /// Per-event mutation for `apply_events`; the caller owns the
    /// `cx.notify()` so a batch renders once, not once per event.
    fn apply_one(&mut self, chat_id: u64, ev: AgentEvent, cx: &mut Context<Self>) {
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
            AgentEvent::RateLimit(rl) if rl.limited => {
                self.agent_log(chat_id, crate::agents::AgentLogEntry { line: "rate limited".into(), count_step: false }, cx);
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
        // A stored allowlist rule answers the request before the card
        // exists — computed here because `chat` borrows `self` below.
        let auto_approved = matches!(&ev, AgentEvent::ApprovalRequest { kind, detail, .. } if self.approval_rule_allows(*kind, detail));
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
                // A matching allowlist rule answers on the spot: the card
                // lands already decided and the blocked backend thread
                // gets its Approve without waiting on a click.
                let (decision, respond) = if auto_approved {
                    let _ = respond.send(crate::backend::ApprovalDecision::Approve);
                    (Some(crate::backend::ApprovalDecision::Approve), None)
                } else {
                    (None, Some(respond))
                };
                Rc::make_mut(&mut chat.messages).push(ChatMessage {
                    role: Role::Assistant,
                    kind: MessageKind::Approval(crate::backend::ApprovalCard {
                        request_ix: ix,
                        kind,
                        detail: detail.clone(),
                        decision,
                        auto_approved,
                        respond,
                    }),
                    rating: None,
                    bookmarked: false,
                    pinned: false,
                    usage: None,
                    attachments: vec![],
                    alternatives: vec![],
                    at: SystemTime::now(),
                });
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
                // The activity feed's approval row means "needs a click" —
                // a rule-answered request never does.
                if !auto_approved {
                    self.record_approval(chat_id, kind, &detail);
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
                // the counters. Token backends carry input/output. The
                // turn's own backend name lives on its agent row — the
                // workspace selection can differ (scheduled prompts run
                // on the chat's stamped provider).
                let occupancy = chat
                    .run_agent
                    .and_then(|id| self.agents.iter().find(|a| a.id == id))
                    .is_some_and(|a| a.name == "acp");
                let report = if occupancy {
                    crate::usage::UsageReport::occupancy(input, output)
                } else {
                    crate::usage::UsageReport::tokens(input, output)
                };
                chat.usage.record(report);
                // Stamp the turn's token total on the reply — the usage
                // dashboard's daily chart reads these persisted stamps.
                // Occupancy reports aren't tokens and stamp nothing.
                if !occupancy {
                    stamp_reply_usage(chat, input, output);
                }
            },
            AgentEvent::RateLimit(rl) => chat.usage.record_rate_limit(rl),
            AgentEvent::ThreadBound(id) => {
                // The backend's thread id — later sends resume it, and the
                // binding must reach disk now so a quit mid-turn still
                // resumes on the next launch.
                if chat.thread_id != id.as_str() {
                    chat.thread_id = id.to_string();
                    self.save();
                }
            },
            AgentEvent::Done => {},
            AgentEvent::Error(msg) => {
                // A throttling error also raises the rate-limit banner —
                // the snapshot merges onto any windows already reported.
                if let Some(rl) = crate::rate_limit::RateLimit::from_error(&msg) {
                    chat.usage.record_rate_limit(rl);
                }
                chat.failed_flag = true;
                push_message(chat, MessageKind::Text(format!("**Error:** {msg}").into()));
                if crate::chat_search::grows_scroller(is_active, chat.messages.last().unwrap(), &query) {
                    self.scroller.update(cx, |s, cx| s.append(1, cx));
                }
            },
        }
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

/// Stamp a turn's token total on its reply — the latest assistant text
/// bubble, falling back to any assistant message. Persisted on the
/// message; the usage dashboard's daily chart buckets these stamps.
fn stamp_reply_usage(chat: &mut crate::model::Chat, input: u64, output: u64) {
    let ix = chat
        .messages
        .iter()
        .rposition(|m| m.role == Role::Assistant && matches!(m.kind, MessageKind::Text(_)))
        .or_else(|| chat.messages.iter().rposition(|m| m.role == Role::Assistant));
    if let Some(ix) = ix {
        Rc::make_mut(&mut chat.messages)[ix].usage = Some(crate::model::Usage { input, output });
    }
}

/// Append an assistant message carrying `kind` to the chat.
fn push_message(chat: &mut crate::model::Chat, kind: MessageKind) {
    let mut msg = ChatMessage {
        role: Role::Assistant,
        kind,
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: SystemTime::now(),
        alternatives: vec![],
    };
    // A regenerate/retry saved the outgoing reply's version chain — the
    // new turn's first text bubble inherits it.
    if matches!(msg.kind, MessageKind::Text(_)) {
        chat.adopt_alternatives(&mut msg);
    }
    Rc::make_mut(&mut chat.messages).push(msg);
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
