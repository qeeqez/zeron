//! `stream_event` decoding for `claude -p --include-partial-messages`:
//! raw API stream events — `content_block_*` deltas render text
//! token-by-token and open live tool/thinking cards. Split from
//! `claude_parse.rs` so both stay under the SLOC cap.

use serde_json::Value;

use super::claude_parse::{Block, ClaudeDecoder};
use crate::backend::AgentEvent;
use crate::backend_parse::item_ix;

impl ClaudeDecoder {
    /// Raw API stream event (`--include-partial-messages`). Indices are
    /// per-message; `message_start` resets the per-message trackers.
    pub(super) fn stream_event(&mut self, ev: &Value) -> Vec<AgentEvent> {
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
                self.blocks.insert(ix, Block::Tool);
                // `TodoWrite` renders as the plan checklist, not a tool
                // card — the input only exists at the `assistant` snapshot.
                if block["name"].as_str() == Some("TodoWrite") {
                    self.todos.insert(id);
                    return vec![];
                }
                let card = item_ix(&serde_json::json!({"id": id}));
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
}
