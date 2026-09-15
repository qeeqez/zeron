//! ACP `session/update` → `AgentEvent` decoder. Tracks open text,
//! thought, and tool cards so streamed updates route to the right
//! message and terminal statuses close cards exactly once.

use serde_json::Value;

use super::AgentEvent;
use crate::backend_parse::plan_steps;

/// Synthetic ids for cards that have no toolCallId of their own.
const PLAN_ID: &str = "__acp_plan__";
const THOUGHT_ID: &str = "__acp_thought__";

/// Per-turn decoder: maps `session/update` payloads to `AgentEvent`s.
pub(super) struct AcpDecoder {
    /// toolCallIds that produced a `ToolCallStart` card already.
    started: std::collections::HashSet<String>,
    /// toolCallIds whose terminal status already emitted `ToolCallEnd`.
    ended: std::collections::HashSet<String>,
    /// messageId of the in-flight agent message — a change opens a fresh
    /// text bubble via `TextStart`.
    message: Option<String>,
    /// messageId of the in-flight thought stream (the "thinking" card).
    thought: Option<String>,
}

impl AcpDecoder {
    pub(super) fn new() -> Self {
        Self {
            started: std::collections::HashSet::new(),
            ended: std::collections::HashSet::new(),
            message: None,
            thought: None,
        }
    }

    /// Decode one `session/update` notification's `update` payload.
    /// Unknown variants are ignored — the agent emits many we don't render.
    pub(super) fn update(&mut self, u: &Value) -> Vec<AgentEvent> {
        match u["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => self.text_chunk(u),
            Some("agent_thought_chunk") => self.thought_chunk(u),
            Some("tool_call") => self.tool_call(u),
            Some("tool_call_update") => self.tool_update(u),
            Some("plan") => self.plan(u),
            Some("usage_update") => vec![self.usage(u)],
            _ => vec![],
        }
    }

    /// `agent_message_chunk`: a new `messageId` starts a fresh bubble.
    /// Non-text blocks render as a bracketed placeholder.
    fn text_chunk(&mut self, u: &Value) -> Vec<AgentEvent> {
        let mut out = vec![];
        self.close_thought_into(&mut out);
        let mid = u["messageId"].as_str().unwrap_or("").to_string();
        if self.message.as_deref() != Some(mid.as_str()) {
            self.message = Some(mid);
            out.push(AgentEvent::TextStart);
        }
        out.push(AgentEvent::TextDelta(content_text(&u["content"]).into()));
        out
    }

    /// `agent_thought_chunk`: streams into a "thinking" tool card keyed by
    /// `messageId`; any other update closes it.
    fn thought_chunk(&mut self, u: &Value) -> Vec<AgentEvent> {
        let mid = u["messageId"].as_str().unwrap_or("").to_string();
        let mut out = vec![];
        if self.thought.as_deref() != Some(mid.as_str()) {
            self.close_thought_into(&mut out);
            self.thought = Some(mid);
            out.push(AgentEvent::ToolCallStart {
                ix: ix_of(THOUGHT_ID),
                name: "thinking".into(),
                detail: "".into(),
            });
        }
        out.push(AgentEvent::ToolCallDelta {
            ix: ix_of(THOUGHT_ID),
            output: content_text(&u["content"]).into(),
        });
        out
    }

    /// `tool_call`: open a card named by the ACP kind, titled by `title`.
    /// A terminal status on arrival closes it immediately.
    fn tool_call(&mut self, u: &Value) -> Vec<AgentEvent> {
        let mut out = vec![];
        self.close_thought_into(&mut out);
        let id = u["toolCallId"].as_str().unwrap_or("").to_string();
        let ix = ix_of(&id);
        if self.started.insert(id.clone()) {
            out.push(AgentEvent::ToolCallStart {
                ix,
                name: kind_name(u["kind"].as_str()).into(),
                detail: u["title"].as_str().unwrap_or("").into(),
            });
        }
        out.extend(self.tool_content(ix, u));
        if let Some(ok) = terminal_ok(u["status"].as_str()) {
            out.extend(self.end_tool(&id, ok));
        }
        out
    }

    /// `tool_call_update`: content replaces (ToolCallSet), a terminal
    /// status ends the card. An update for an unseen id opens the card
    /// first — agents may skip the initial `tool_call`.
    fn tool_update(&mut self, u: &Value) -> Vec<AgentEvent> {
        let id = u["toolCallId"].as_str().unwrap_or("").to_string();
        let mut out = vec![];
        self.close_thought_into(&mut out);
        if self.ended.contains(&id) {
            return out;
        }
        let ix = ix_of(&id);
        if self.started.insert(id.clone()) {
            out.push(AgentEvent::ToolCallStart {
                ix,
                name: kind_name(u["kind"].as_str()).into(),
                detail: u["title"].as_str().unwrap_or("").into(),
            });
        }
        out.extend(self.tool_content(ix, u));
        if let Some(ok) = terminal_ok(u["status"].as_str()) {
            out.extend(self.end_tool(&id, ok));
        }
        out
    }

    /// Render a tool call's `content` array: text blocks become output,
    /// diffs become `Diff` cards. `rawOutput` fills in when no content
    /// was sent. Returns `ToolCallSet` (content is a full snapshot).
    fn tool_content(&mut self, ix: usize, u: &Value) -> Vec<AgentEvent> {
        let mut out = vec![];
        let mut text = String::new();
        for c in u["content"].as_array().into_iter().flatten() {
            if c["type"].as_str() == Some("diff") {
                out.push(diff_event(c));
                continue;
            }
            let t = content_text(&c["content"]);
            if t.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&t);
        }
        if text.is_empty()
            && let Some(raw) = u["rawOutput"].as_str().filter(|r| !r.is_empty())
        {
            text = raw.to_string();
        }
        if !text.is_empty() {
            out.insert(0, AgentEvent::ToolCallSet { ix, output: text.into() });
        }
        out
    }

    /// `plan`: the whole checklist arrives each time — replace the card.
    /// Plan cards have no spinner, so nothing opens or closes them.
    fn plan(&mut self, u: &Value) -> Vec<AgentEvent> {
        let mut out = vec![];
        self.close_thought_into(&mut out);
        let steps = plan_steps(&u["entries"], "content", "status");
        if !steps.is_empty() {
            out.push(AgentEvent::Plan { ix: ix_of(PLAN_ID), steps });
        }
        out
    }

    /// `usage_update` reports context occupancy (`used` of `size`), not
    /// per-turn input/output — surface it as `used in · size out`.
    fn usage(&self, u: &Value) -> AgentEvent {
        AgentEvent::Usage {
            input: u["used"].as_u64().unwrap_or(0),
            output: u["size"].as_u64().unwrap_or(0),
        }
    }

    /// Emit `ToolCallEnd` for a terminal status, once per tool call.
    fn end_tool(&mut self, id: &str, ok: bool) -> Vec<AgentEvent> {
        if self.ended.insert(id.to_string()) {
            vec![AgentEvent::ToolCallEnd { ix: ix_of(id), ok }]
        } else {
            vec![]
        }
    }

    /// Close the in-flight thought card, if any.
    fn close_thought_into(&mut self, out: &mut Vec<AgentEvent>) {
        if self.thought.take().is_some() {
            out.push(AgentEvent::ToolCallEnd { ix: ix_of(THOUGHT_ID), ok: true });
        }
    }

    /// Turn end: close any cards still open so nothing spins forever.
    /// Plan cards need no close — they render their last snapshot.
    pub(super) fn close_open(&mut self) -> Vec<AgentEvent> {
        let mut out = vec![];
        self.close_thought_into(&mut out);
        let open: Vec<String> = self.started.difference(&self.ended).cloned().collect();
        self.started.clear();
        for id in open {
            out.push(AgentEvent::ToolCallEnd { ix: ix_of(&id), ok: true });
        }
        out
    }
}

/// `ok` for a terminal ACP tool status, `None` while still running.
fn terminal_ok(status: Option<&str>) -> Option<bool> {
    match status {
        Some("completed") => Some(true),
        Some("failed") => Some(false),
        _ => None,
    }
}

/// Stable per-tool-call card index — hash the id like the codex decoder.
fn ix_of(id: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish() as usize
}

/// Display name for an ACP tool kind.
fn kind_name(kind: Option<&str>) -> &'static str {
    match kind {
        Some("read") => "read",
        Some("edit") => "edit",
        Some("delete") => "delete",
        Some("move") => "move",
        Some("search") => "search",
        Some("execute") => "shell",
        Some("think") => "thinking",
        Some("fetch") => "fetch",
        Some("switch_mode") => "mode",
        _ => "tool",
    }
}

/// Text of a content block; non-text blocks get a bracketed placeholder.
fn content_text(block: &Value) -> String {
    match block["type"].as_str() {
        Some("text") => block["text"].as_str().unwrap_or("").to_string(),
        Some(other) => format!("[{other}]"),
        None => String::new(),
    }
}

/// An ACP `diff` tool content block → a `Diff` card. The protocol carries
/// full old/new text rather than hunks, so the card shows a synthesized
/// whole-file diff (old lines `-`, new lines `+`).
fn diff_event(c: &Value) -> AgentEvent {
    let old = c["oldText"].as_str().unwrap_or("");
    let new = c["newText"].as_str().unwrap_or("");
    let mut hunks = format!("@@ {} @@\n", c["path"].as_str().unwrap_or(""));
    for l in old.lines() {
        hunks.push_str(&format!("-{l}\n"));
    }
    for l in new.lines() {
        hunks.push_str(&format!("+{l}\n"));
    }
    AgentEvent::Diff {
        path: c["path"].as_str().unwrap_or("").into(),
        added: new.lines().count(),
        removed: old.lines().count(),
        hunks: hunks.into(),
    }
}
