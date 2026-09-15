//! `codex app-server` protocol: map NDJSON-RPC notifications to `AgentEvent`s.
//!
//! Unlike `codex exec --json` (which only reports completed items), the
//! app-server streams `item/agentMessage/delta` and
//! `item/commandExecution/outputDelta`, so text and tool output render live —
//! the same transport the Codex desktop app uses.

use serde_json::{Value, json};

use super::rpc::request_reply;
use crate::backend::AgentEvent;
use crate::backend_parse::{file_change_events, item_id, item_ix, mcp_result_text, plan_steps, plan_steps_from_text, reasoning_text};

/// Item id of the synthetic plan card — `turn/plan/updated` has no item
/// id of its own, so the checklist lives under this key.
const PLAN_ID: &str = "__turn_plan__";

/// Per-turn decoder: tracks which items have started/streamed so completed
/// payloads don't double-emit what deltas already delivered.
pub(crate) struct TurnDecoder {
    /// Item ids that produced a `ToolCallStart`/`TextStart` card already.
    started: std::collections::HashSet<String>,
    /// Item ids whose content arrived via deltas — `item/completed` must not
    /// re-emit the full payload.
    streamed: std::collections::HashSet<String>,
    /// Synthetic item used to key the live plan checklist card.
    plan_item: Value,
    /// Set once a terminal error was emitted so `turn/completed` doesn't
    /// push a second error bubble.
    errored: bool,
}

/// One decoded line: events for the UI plus an optional JSON-RPC response
/// that must be written back to the server's stdin (server requests).
pub(crate) struct Decoded {
    pub events: Vec<AgentEvent>,
    pub response: Option<Value>,
    /// `turn/completed` arrived — the turn is over regardless of status.
    pub turn_over: bool,
}

impl TurnDecoder {
    pub fn new() -> Self {
        Self {
            started: std::collections::HashSet::new(),
            streamed: std::collections::HashSet::new(),
            plan_item: json!({"id": PLAN_ID}),
            errored: false,
        }
    }

    /// Decode one stdout line. Malformed JSON and unknown methods are
    /// ignored — the server emits many notifications we don't render.
    pub fn line(&mut self, line: &str) -> Decoded {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return Decoded { events: vec![], response: None, turn_over: false };
        };
        // Responses to our own requests are handled by the caller's phase
        // machine; here only server-initiated messages matter.
        let Some(method) = msg["method"].as_str() else {
            return Decoded { events: vec![], response: None, turn_over: false };
        };
        if msg.get("id").is_some() {
            return Decoded {
                events: vec![],
                response: Some(request_reply(method, &msg)),
                turn_over: false,
            };
        }
        let params = &msg["params"];
        let (events, turn_over) = self.notification(method, params);
        Decoded { events, response: None, turn_over }
    }

    fn notification(&mut self, method: &str, params: &Value) -> (Vec<AgentEvent>, bool) {
        match method {
            "item/started" => (self.item_started(&params["item"]), false),
            "item/completed" => (self.item_completed(&params["item"]), false),
            "item/agentMessage/delta" => (self.delta(params, DeltaKind::Text), false),
            "item/commandExecution/outputDelta" => (self.delta(params, DeltaKind::Tool("shell")), false),
            "item/fileChange/outputDelta" => (self.delta(params, DeltaKind::Tool("file_change")), false),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => (self.delta(params, DeltaKind::Tool("thinking")), false),
            "item/mcpToolCall/progress" => (self.progress(params), false),
            "turn/plan/updated" => (self.plan_updated(params), false),
            "thread/tokenUsage/updated" => (self.usage(params), false),
            "error" => (self.error(params), false),
            "turn/completed" => (self.turn_completed(&params["turn"]), true),
            _ => (vec![], false),
        }
    }

    /// `item/started`: open the right card for the item kind. Agent messages
    /// get a fresh text bubble; tool-ish items get a running tool card.
    fn item_started(&mut self, item: &Value) -> Vec<AgentEvent> {
        let id = item_id(item);
        match item["type"].as_str() {
            Some("agentMessage") => {
                self.started.insert(id);
                vec![AgentEvent::TextStart]
            },
            Some("commandExecution") => self.tool_start(item, "shell", item["command"].as_str().unwrap_or("")),
            Some("reasoning") => self.tool_start(item, "thinking", ""),
            Some("mcpToolCall") => self.tool_start(item, item["server"].as_str().unwrap_or("mcp"), item["tool"].as_str().unwrap_or("")),
            Some("dynamicToolCall") => self.tool_start(item, "tool", item["tool"].as_str().unwrap_or("")),
            Some("collabAgentToolCall") => self.tool_start(item, "agent", item["tool"].as_str().unwrap_or("")),
            Some("webSearch") => self.tool_start(item, "web_search", item["query"].as_str().unwrap_or("")),
            Some("fileChange") => self.tool_start(item, "file_change", ""),
            // `plan` items carry the proposed plan as markdown — the card
            // opens on `item/completed` once the text is known.
            Some("plan") => vec![],
            _ => vec![],
        }
    }

    /// `item/completed`: flush any content that didn't stream, then close
    /// the card. Agent messages only emit text when no deltas arrived.
    fn item_completed(&mut self, item: &Value) -> Vec<AgentEvent> {
        let id = item_id(item);
        let ix = item_ix(item);
        match item["type"].as_str() {
            Some("agentMessage") => {
                let text = item["text"].as_str().unwrap_or("");
                if self.streamed.contains(&id) || text.is_empty() {
                    vec![]
                } else {
                    vec![AgentEvent::TextDelta(text.into())]
                }
            },
            Some("commandExecution") => {
                let mut out = self.tool_start(item, "shell", item["command"].as_str().unwrap_or(""));
                let output = item["aggregatedOutput"].as_str().unwrap_or("");
                if !self.streamed.contains(&id) && !output.is_empty() {
                    out.push(AgentEvent::ToolCallDelta { ix, output: output.into() });
                }
                out.push(AgentEvent::ToolCallEnd { ix, ok: item["status"].as_str() == Some("completed") });
                out
            },
            Some("reasoning") => {
                let mut out = self.tool_start(item, "thinking", "");
                let text = if self.streamed.contains(&id) { String::new() } else { reasoning_text(item) };
                if !text.is_empty() {
                    out.push(AgentEvent::ToolCallDelta { ix, output: text.into() });
                }
                out.push(AgentEvent::ToolCallEnd { ix, ok: true });
                out
            },
            Some("mcpToolCall") | Some("dynamicToolCall") | Some("collabAgentToolCall") => {
                let mut out = vec![];
                let result = if self.streamed.contains(&id) { String::new() } else { mcp_result_text(item) };
                if !result.is_empty() {
                    out.push(AgentEvent::ToolCallDelta { ix, output: result.into() });
                }
                out.push(AgentEvent::ToolCallEnd { ix, ok: item["status"].as_str() == Some("completed") });
                out
            },
            Some("webSearch") => vec![AgentEvent::ToolCallEnd { ix, ok: true }],
            Some("fileChange") => {
                let mut out = file_change_events(item);
                out.push(AgentEvent::ToolCallEnd { ix, ok: item["status"].as_str() == Some("completed") });
                out
            },
            Some("plan") => self.plan_item_completed(item),
            _ => vec![],
        }
    }

    /// A completed `plan` item: markdown checklists become `Plan` steps;
    /// prose plans keep the old text card (open + fill + close in one go —
    /// `item/started` doesn't open a card for plan items).
    fn plan_item_completed(&mut self, item: &Value) -> Vec<AgentEvent> {
        let text = item["text"].as_str().unwrap_or("");
        if let Some(steps) = plan_steps_from_text(text) {
            return vec![AgentEvent::Plan { ix: item_ix(item), steps }];
        }
        if text.is_empty() {
            return vec![];
        }
        let ix = item_ix(item);
        vec![
            AgentEvent::ToolCallStart { ix, name: "plan".into(), detail: "".into() },
            AgentEvent::ToolCallSet { ix, output: text.into() },
            AgentEvent::ToolCallEnd { ix, ok: true },
        ]
    }

    /// A content delta for an item: text for agent messages, output for
    /// tool cards. Opens the card first when `item/started` was skipped.
    fn delta(&mut self, params: &Value, kind: DeltaKind) -> Vec<AgentEvent> {
        let id = params["itemId"].as_str().unwrap_or("").to_string();
        let delta = params["delta"].as_str().unwrap_or("");
        if delta.is_empty() {
            return vec![];
        }
        self.streamed.insert(id.clone());
        let item = json!({"id": id});
        let ix = item_ix(&item);
        let mut out = vec![];
        if self.started.insert(id) {
            out.push(match kind {
                DeltaKind::Text => AgentEvent::TextStart,
                DeltaKind::Tool(name) => AgentEvent::ToolCallStart { ix, name: name.into(), detail: "".into() },
            });
        }
        out.push(match kind {
            DeltaKind::Text => AgentEvent::TextDelta(delta.into()),
            DeltaKind::Tool(_) => AgentEvent::ToolCallDelta { ix, output: delta.into() },
        });
        out
    }

    /// MCP progress lines append to the tool card's output.
    fn progress(&mut self, params: &Value) -> Vec<AgentEvent> {
        let Some(message) = params["message"].as_str().filter(|m| !m.is_empty()) else { return vec![] };
        let item = json!({"id": params["itemId"].as_str().unwrap_or("")});
        vec![AgentEvent::ToolCallDelta { ix: item_ix(&item), output: format!("{message}\n").into() }]
    }

    /// `turn/plan/updated` carries the whole checklist — replace the plan
    /// card's steps rather than appending.
    fn plan_updated(&mut self, params: &Value) -> Vec<AgentEvent> {
        let steps = plan_steps(&params["plan"], "step", "status");
        if steps.is_empty() {
            return vec![];
        }
        vec![AgentEvent::Plan { ix: item_ix(&self.plan_item), steps }]
    }

    /// Live token usage for the in-flight turn (`last` is this turn's
    /// slice; `total` is cumulative for the thread).
    fn usage(&self, params: &Value) -> Vec<AgentEvent> {
        let last = &params["tokenUsage"]["last"];
        vec![AgentEvent::Usage {
            input: last["inputTokens"].as_u64().unwrap_or(0),
            output: last["outputTokens"].as_u64().unwrap_or(0),
        }]
    }

    /// Stream errors with `willRetry` are transient — the server retries
    /// internally, so surfacing them would flash a bogus failure banner.
    fn error(&mut self, params: &Value) -> Vec<AgentEvent> {
        if params["willRetry"].as_bool().unwrap_or(false) {
            return vec![];
        }
        let err = &params["error"];
        let message = err["message"].as_str().unwrap_or("codex error");
        let detail = err["additionalDetails"].as_str().filter(|d| !d.is_empty());
        let text = detail.map_or_else(|| message.to_string(), |d| format!("{message} ({d})"));
        self.errored = true;
        vec![AgentEvent::Error(text.into())]
    }

    /// `turn/completed` ends the turn. `failed` surfaces the turn error
    /// (unless one was already emitted); `interrupted` is a clean stop —
    /// the user cancelled, so no error bubble. Plan cards keep their last
    /// snapshot — the checklist has no spinner to settle.
    fn turn_completed(&mut self, turn: &Value) -> Vec<AgentEvent> {
        let mut out = vec![];
        match turn["status"].as_str() {
            Some("failed") if !self.errored => {
                let err = &turn["error"];
                let message = err["message"].as_str().unwrap_or("turn failed");
                let detail = err["additionalDetails"].as_str().filter(|d| !d.is_empty());
                let text = detail.map_or_else(|| message.to_string(), |d| format!("{message} ({d})"));
                out.push(AgentEvent::Error(text.into()));
            },
            _ => {},
        }
        out.push(AgentEvent::Done);
        out
    }

    /// Emit `ToolCallStart` unless this item already opened a card.
    fn tool_start(&mut self, item: &Value, name: &str, detail: &str) -> Vec<AgentEvent> {
        if self.started.insert(item_id(item)) {
            vec![AgentEvent::ToolCallStart { ix: item_ix(item), name: name.into(), detail: detail.into() }]
        } else {
            vec![]
        }
    }
}

enum DeltaKind {
    Text,
    /// Tool output; the name labels the card if the delta opens it before
    /// `item/started` arrives.
    Tool(&'static str),
}
