//! `claude -p --output-format stream-json` protocol: map NDJSON events to
//! `AgentEvent`s.
//!
//! With `--include-partial-messages` the CLI wraps raw API stream events in
//! `{"type":"stream_event","event":{…}}` lines — `content_block_delta` gives
//! token-by-token text, so replies render live. The full `assistant` message
//! still follows each model response; blocks already delivered by deltas are
//! skipped there so nothing double-prints.

use serde_json::Value;

use crate::backend::AgentEvent;
use crate::backend_parse::{item_ix, result_text, todo_steps};

/// Synthetic card ix for `TodoWrite` checklists — every todo write updates
/// the one plan card rather than opening a card per tool call.
const PLAN_ID: &str = "__claude_plan__";

/// Per-turn decoder: tracks which blocks streamed so the `assistant`
/// snapshot doesn't re-emit them, and which tool cards are still open.
/// `pub(super)` fields serve the stream half in `claude_parse_stream`.
pub(crate) struct ClaudeDecoder {
    /// tool_use ids that produced a `ToolCallStart` card already.
    pub(super) started: std::collections::HashSet<String>,
    /// tool_use ids whose card is still open — closed by `tool_result` or,
    /// as a fallback, by `result` so an interrupted turn can't spin forever.
    pub(super) pending: std::collections::HashSet<String>,
    /// Content-block indices (this message) whose text arrived via deltas.
    pub(super) text_streamed: std::collections::HashSet<i64>,
    /// Live stream blocks by content index — `content_block_stop` needs the
    /// card ix for thinking/tool blocks.
    pub(super) blocks: std::collections::HashMap<i64, Block>,
    /// tool_use ids of `TodoWrite` calls — their `tool_result` carries no
    /// card output, so `user` skips them.
    pub(super) todos: std::collections::HashSet<String>,
    /// Latest token counts — `message_start`/`message_delta` carry partial
    /// usage, `result` carries the final totals.
    last_usage: (u64, u64),
    /// The session id once seen — every stream frame carries it; the first
    /// sighting emits `ThreadBound` so the chat resumes this session on
    /// later sends.
    session_id: Option<String>,
}

/// What a live `content_block` turned out to be — only thinking blocks
/// need their card ix remembered for `content_block_stop`; tool cards
/// close via `pending` ids when `tool_result` arrives.
pub(super) enum Block {
    Text,
    Thinking(usize),
    Tool,
}

/// One decoded line: events for the UI plus whether the turn ended.
pub(crate) struct ClaudeDecoded {
    pub events: Vec<AgentEvent>,
    /// `result` arrived — the process is about to exit.
    pub turn_over: bool,
}

impl ClaudeDecoder {
    pub fn new() -> Self {
        Self {
            started: std::collections::HashSet::new(),
            pending: std::collections::HashSet::new(),
            text_streamed: std::collections::HashSet::new(),
            blocks: std::collections::HashMap::new(),
            todos: std::collections::HashSet::new(),
            last_usage: (0, 0),
            session_id: None,
        }
    }

    /// Decode one stdout line. Malformed JSON and unknown types are
    /// ignored — the stream carries bookkeeping (`system`, `rate_limit`)
    /// we don't render. The first frame carrying `session_id` binds the
    /// chat's thread so later sends resume this session.
    pub fn line(&mut self, line: &str) -> ClaudeDecoded {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return ClaudeDecoded { events: vec![], turn_over: false };
        };
        let bound = self.session_binding(&msg);
        let mut events = match msg["type"].as_str() {
            Some("stream_event") => self.stream_event(&msg["event"]),
            Some("assistant") => self.assistant(&msg["message"]),
            Some("user") => self.user(&msg["message"]),
            Some("result") => {
                let mut out = self.result(&msg);
                out.extend(bound);
                return ClaudeDecoded { events: out, turn_over: true };
            },
            // `rate_limit_event` frames carry quota status — "rejected"
            // raises the banner, "allowed" clears it.
            Some("rate_limit_event") => crate::rate_limit::RateLimit::from_claude(&msg["rate_limit_info"])
                .map(|rl| vec![AgentEvent::RateLimit(rl)])
                .unwrap_or_default(),
            _ => vec![],
        };
        events.extend(bound);
        ClaudeDecoded { events, turn_over: false }
    }

    /// The first frame carrying `session_id` binds the chat's thread so
    /// later sends resume this session — every frame repeats the id, so
    /// only a new sighting emits `ThreadBound`.
    fn session_binding(&mut self, msg: &Value) -> Option<AgentEvent> {
        let sid = msg["session_id"].as_str().filter(|s| !s.is_empty())?;
        if self.session_id.as_deref() == Some(sid) {
            return None;
        }
        self.session_id = Some(sid.to_string());
        Some(AgentEvent::ThreadBound(sid.into()))
    }

    /// Full assistant message after the model response. Blocks already
    /// delivered by deltas are skipped; without partials this is where all
    /// text and tool cards come from.
    fn assistant(&mut self, message: &Value) -> Vec<AgentEvent> {
        let mut out = self.usage_events(&message["usage"]);
        let Some(content) = message["content"].as_array() else { return out };
        for (i, block) in content.iter().enumerate() {
            out.extend(self.snapshot_block(i as i64, block));
        }
        out
    }

    /// One content block of the `assistant` snapshot.
    fn snapshot_block(&mut self, i: i64, block: &Value) -> Vec<AgentEvent> {
        match block["type"].as_str() {
            Some("text") => {
                let text = block["text"].as_str().unwrap_or("");
                if text.is_empty() || self.text_streamed.contains(&i) {
                    vec![]
                } else {
                    vec![AgentEvent::TextDelta(text.into())]
                }
            },
            Some("thinking") => {
                let text = block["thinking"].as_str().unwrap_or("");
                let id = format!("thinking-{i}");
                if text.is_empty() || !self.started.insert(id.clone()) {
                    return vec![];
                }
                let card = item_ix(&serde_json::json!({"id": id}));
                vec![
                    AgentEvent::ToolCallStart { ix: card, name: "thinking".into(), detail: "".into() },
                    AgentEvent::ToolCallDelta { ix: card, output: text.into() },
                    AgentEvent::ToolCallEnd { ix: card, ok: true },
                ]
            },
            Some("tool_use") => self.tool_use(block),
            _ => vec![],
        }
    }
    /// `tool_use` block in the `assistant` snapshot. When the stream
    /// already opened the card, the input summary goes to the card's
    /// output — `ToolCallStart.detail` was empty at `content_block_start`
    /// because the input only existed as partial JSON then. `TodoWrite`
    /// becomes the plan checklist instead of a tool card.
    fn tool_use(&mut self, block: &Value) -> Vec<AgentEvent> {
        let id = block["id"].as_str().unwrap_or("").to_string();
        let name = block["name"].as_str().unwrap_or("tool");
        if name == "TodoWrite" {
            self.todos.insert(id);
            let steps = todo_steps(&block["input"]);
            return if steps.is_empty() {
                vec![]
            } else {
                vec![AgentEvent::Plan { ix: item_ix(&serde_json::json!({"id": PLAN_ID})), steps }]
            };
        }
        let detail = tool_detail(name, &block["input"]);
        let card = item_ix(&serde_json::json!({"id": id}));
        if !self.started.insert(id.clone()) {
            return if detail.is_empty() {
                vec![]
            } else {
                vec![AgentEvent::ToolCallDelta { ix: card, output: format!("{detail}\n").into() }]
            };
        }
        self.pending.insert(id);
        vec![AgentEvent::ToolCallStart { ix: card, name: name.into(), detail: detail.into() }]
    }

    /// `user` message: tool results. Each `tool_result` closes the card its
    /// `tool_use` opened — or opens+closes one when the start was missed.
    fn user(&mut self, message: &Value) -> Vec<AgentEvent> {
        let mut out = vec![];
        let Some(content) = message["content"].as_array() else { return out };
        for block in content {
            if block["type"].as_str() != Some("tool_result") {
                continue;
            }
            let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
            // `TodoWrite` results carry no card output — the plan card
            // already shows the checklist.
            if self.todos.remove(&id) {
                continue;
            }
            self.pending.remove(&id);
            let card = item_ix(&serde_json::json!({"id": id}));
            if self.started.insert(id) {
                out.push(AgentEvent::ToolCallStart { ix: card, name: "tool".into(), detail: "".into() });
            }
            let text = result_text(&block["content"]);
            if !text.is_empty() {
                out.push(AgentEvent::ToolCallDelta { ix: card, output: text.into() });
            }
            out.push(AgentEvent::ToolCallEnd { ix: card, ok: !block["is_error"].as_bool().unwrap_or(false) });
        }
        out
    }

    /// `result` is the last line: final usage, then Done — or Error for the
    /// `error_*` subtypes. Open tool cards are closed first so a turn that
    /// ended mid-tool can't leave a spinner.
    fn result(&mut self, msg: &Value) -> Vec<AgentEvent> {
        let ok = msg["subtype"].as_str() == Some("success") && !msg["is_error"].as_bool().unwrap_or(false);
        let mut out: Vec<AgentEvent> = self
            .pending
            .drain()
            .map(|id| AgentEvent::ToolCallEnd { ix: item_ix(&serde_json::json!({"id": id})), ok })
            .collect();
        out.extend(self.usage_events(&msg["usage"]));
        if !ok {
            let subtype = msg["subtype"].as_str().unwrap_or("error");
            let text = msg["result"].as_str().filter(|r| !r.is_empty());
            let err = text.map_or_else(|| format!("claude: {subtype}"), |r| format!("claude {subtype}: {r}"));
            out.push(AgentEvent::Error(err.into()));
        }
        out.push(AgentEvent::Done);
        out
    }

    /// Emit `Usage` when the payload carries token counts. Input totals
    /// fold in cache creation/read tokens — they're billed input.
    pub(super) fn usage_events(&mut self, usage: &Value) -> Vec<AgentEvent> {
        let input = usage["input_tokens"].as_u64().unwrap_or(0)
            + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)
            + usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let output = usage["output_tokens"].as_u64().unwrap_or(0);
        if input > 0 {
            self.last_usage.0 = input;
        }
        if output > 0 {
            self.last_usage.1 = output;
        }
        if self.last_usage == (0, 0) {
            vec![]
        } else {
            vec![AgentEvent::Usage { input: self.last_usage.0, output: self.last_usage.1 }]
        }
    }
}

/// One-line summary of a tool call's input for the card header — the
/// interesting field per tool, not the whole JSON blob.
fn tool_detail(name: &str, input: &Value) -> String {
    let key = match name {
        "Bash" => "command",
        "Read" | "Write" | "Edit" | "NotebookEdit" => "file_path",
        "Glob" | "Grep" => "pattern",
        "WebFetch" => "url",
        "WebSearch" => "query",
        _ => "",
    };
    input[key].as_str().unwrap_or("").to_string()
}
