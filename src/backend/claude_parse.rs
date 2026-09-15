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
use crate::backend_parse::item_ix;

/// Per-turn decoder: tracks which blocks streamed so the `assistant`
/// snapshot doesn't re-emit them, and which tool cards are still open.
pub(crate) struct ClaudeDecoder {
    /// tool_use ids that produced a `ToolCallStart` card already.
    started: std::collections::HashSet<String>,
    /// tool_use ids whose card is still open — closed by `tool_result` or,
    /// as a fallback, by `result` so an interrupted turn can't spin forever.
    pending: std::collections::HashSet<String>,
    /// Content-block indices (this message) whose text arrived via deltas.
    text_streamed: std::collections::HashSet<i64>,
    /// Live stream blocks by content index — `content_block_stop` needs the
    /// card ix for thinking/tool blocks.
    blocks: std::collections::HashMap<i64, Block>,
    /// Latest token counts — `message_start`/`message_delta` carry partial
    /// usage, `result` carries the final totals.
    last_usage: (u64, u64),
}

/// What a live `content_block` turned out to be — only thinking blocks
/// need their card ix remembered for `content_block_stop`; tool cards
/// close via `pending` ids when `tool_result` arrives.
enum Block {
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
            last_usage: (0, 0),
        }
    }

    /// Decode one stdout line. Malformed JSON and unknown types are
    /// ignored — the stream carries bookkeeping (`system`, `rate_limit`)
    /// we don't render.
    pub fn line(&mut self, line: &str) -> ClaudeDecoded {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return ClaudeDecoded { events: vec![], turn_over: false };
        };
        let events = match msg["type"].as_str() {
            Some("stream_event") => self.stream_event(&msg["event"]),
            Some("assistant") => self.assistant(&msg["message"]),
            Some("user") => self.user(&msg["message"]),
            Some("result") => return ClaudeDecoded { events: self.result(&msg), turn_over: true },
            _ => vec![],
        };
        ClaudeDecoded { events, turn_over: false }
    }

    /// Raw API stream event (`--include-partial-messages`). Indices are
    /// per-message; `message_start` resets the per-message trackers.
    fn stream_event(&mut self, ev: &Value) -> Vec<AgentEvent> {
        match ev["type"].as_str() {
            Some("message_start") => self.message_start(&ev["message"]),
            Some("content_block_start") => self.block_start(ev["index"].as_i64().unwrap_or(0), &ev["content_block"]),
            Some("content_block_delta") => self.block_delta(ev["index"].as_i64().unwrap_or(0), &ev["delta"]),
            Some("content_block_stop") => self.block_stop(ev["index"].as_i64().unwrap_or(0)),
            Some("message_delta") => self.usage_events(&ev["usage"]),
            _ => vec![],
        }
    }

    fn message_start(&mut self, message: &Value) -> Vec<AgentEvent> {
        self.blocks.clear();
        self.text_streamed.clear();
        self.usage_events(&message["usage"])
    }

    /// `content_block_start`: open the right card for the block kind.
    /// Text blocks get a fresh bubble; tool_use opens a running tool card.
    fn block_start(&mut self, ix: i64, block: &Value) -> Vec<AgentEvent> {
        match block["type"].as_str() {
            Some("text") => {
                self.blocks.insert(ix, Block::Text);
                vec![AgentEvent::TextStart]
            },
            Some("thinking") => {
                let card = item_ix(&serde_json::json!({"id": format!("thinking-{ix}")}));
                self.blocks.insert(ix, Block::Thinking(card));
                vec![AgentEvent::ToolCallStart { ix: card, name: "thinking".into(), detail: "".into() }]
            },
            Some("tool_use") => {
                let id = block["id"].as_str().unwrap_or("").to_string();
                let card = item_ix(&serde_json::json!({"id": id}));
                self.blocks.insert(ix, Block::Tool);
                self.started.insert(id.clone());
                self.pending.insert(id);
                let name = block["name"].as_str().unwrap_or("tool");
                vec![AgentEvent::ToolCallStart { ix: card, name: name.into(), detail: "".into() }]
            },
            _ => vec![],
        }
    }

    /// `content_block_delta`: text deltas append to the bubble; tool input
    /// streams as partial JSON we don't render (the `assistant` snapshot
    /// summarizes it); thinking deltas append to the thinking card.
    fn block_delta(&mut self, ix: i64, delta: &Value) -> Vec<AgentEvent> {
        match delta["type"].as_str() {
            Some("text_delta") => {
                let text = delta["text"].as_str().unwrap_or("");
                if text.is_empty() {
                    vec![]
                } else {
                    self.text_streamed.insert(ix);
                    vec![AgentEvent::TextDelta(text.into())]
                }
            },
            Some("thinking_delta") => {
                let text = delta["thinking"].as_str().unwrap_or("");
                match (self.blocks.get(&ix), text.is_empty()) {
                    (Some(Block::Thinking(card)), false) => {
                        vec![AgentEvent::ToolCallDelta { ix: *card, output: text.into() }]
                    },
                    _ => vec![],
                }
            },
            _ => vec![],
        }
    }

    /// `content_block_stop`: thinking cards close here; tool_use cards stay
    /// open until `tool_result` arrives in the next `user` message.
    fn block_stop(&mut self, ix: i64) -> Vec<AgentEvent> {
        match self.blocks.remove(&ix) {
            Some(Block::Thinking(card)) => vec![AgentEvent::ToolCallEnd { ix: card, ok: true }],
            _ => vec![],
        }
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
    /// because the input only existed as partial JSON then.
    fn tool_use(&mut self, block: &Value) -> Vec<AgentEvent> {
        let id = block["id"].as_str().unwrap_or("").to_string();
        let name = block["name"].as_str().unwrap_or("tool");
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
    fn usage_events(&mut self, usage: &Value) -> Vec<AgentEvent> {
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

/// `tool_result.content` is a string or an array of content blocks —
/// flatten to displayable text.
fn result_text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let Some(blocks) = content.as_array() else { return String::new() };
    blocks
        .iter()
        .filter_map(|b| match b["type"].as_str() {
            Some("text") => Some(b["text"].as_str().unwrap_or("").to_string()),
            Some("image") => Some("[image]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
