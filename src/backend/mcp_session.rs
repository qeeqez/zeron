//! MCP session: the request/response loop over a server's stdio pipes.
//! `Session` owns the reader/writer pair and the request-id sequence —
//! tests drive it with in-memory cursors, the backend with a spawned
//! child's pipes. Server-initiated requests get a canned reply inline;
//! notifications while a call is live surface through `on_event`.

use std::io::{BufRead, BufReader, Write};

use serde_json::Value;

use super::mcp_rpc as wire;
use super::{AgentEvent, mcp_decode};

/// How a session exchange ended — the caller maps this onto events.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(super) enum SessionErr {
    /// The event receiver or server stdin is gone — stop quietly.
    Dead,
    /// stdout hit EOF before a response arrived — killed or crashed.
    Eof,
    /// Protocol or I/O failure worth an `Error` event.
    Failed(String),
}

type SessionResult<T> = Result<T, SessionErr>;

fn failed<T>(msg: impl Into<String>) -> SessionResult<T> {
    Err(SessionErr::Failed(msg.into()))
}

/// One MCP session over a server's pipes: request ids, the read loop,
/// and the live tool-call index notifications attach to.
pub(super) struct Session<'a> {
    reader: BufReader<Box<dyn std::io::Read + 'a>>,
    stdin: Box<dyn Write + 'a>,
    seq: i64,
    /// The in-flight `tools/call`'s card index — progress/log
    /// notifications attach to it. `None` outside a call.
    call_ix: Option<usize>,
}

impl<'a> Session<'a> {
    pub(super) fn new(reader: impl std::io::Read + 'a, stdin: impl Write + 'a) -> Self {
        Self {
            reader: BufReader::new(Box::new(reader)),
            stdin: Box::new(stdin),
            seq: 0,
            call_ix: None,
        }
    }

    fn next_id(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    /// Send `req`, then read until its response arrives. Server requests
    /// get `wire::server_reply`; notifications while a call is live go to
    /// `on_event` (returning false ends the session `Dead`). `Err(Eof)`
    /// when stdout closes mid-request.
    pub(super) fn request(&mut self, req: &Value, on_event: &mut dyn FnMut(AgentEvent) -> bool) -> SessionResult<Value> {
        let id = req["id"].as_i64().unwrap_or_default();
        wire::send(&mut *self.stdin, req).map_err(|_| SessionErr::Dead)?;
        loop {
            let Some(line) = (&mut self.reader).lines().next() else {
                return Err(SessionErr::Eof);
            };
            let line = line.map_err(|e| SessionErr::Failed(format!("mcp stdout: {e}")))?;
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(result) = self.dispatch(&msg, id, on_event)? {
                return Ok(result);
            }
        }
    }

    /// One inbound message while a request is in flight: `Some(result)`
    /// when it's our response, `None` when it was handled inline (a
    /// server request replied to, a notification forwarded, a stray
    /// response ignored).
    fn dispatch(&mut self, msg: &Value, id: i64, on_event: &mut dyn FnMut(AgentEvent) -> bool) -> SessionResult<Option<Value>> {
        match (msg.get("method").is_some(), msg.get("id")) {
            // A response to our request — `id` matches by construction
            // (the server answers in order on one pipe).
            (false, Some(rid)) if rid.as_i64() == Some(id) => {
                if let Some(err) = msg.get("error") {
                    let m = err["message"].as_str().unwrap_or("request failed");
                    return failed(format!("mcp: {m}"));
                }
                Ok(Some(msg["result"].clone()))
            },
            // A server-initiated request — reply so it can't hang.
            (true, Some(_)) => {
                wire::send(&mut *self.stdin, &wire::server_reply(msg)).map_err(|_| SessionErr::Dead)?;
                Ok(None)
            },
            // A notification — forward while a tool call is live.
            (true, None) => {
                if let Some(e) = self.call_ix.and_then(|ix| mcp_decode::notification(msg, ix))
                    && !on_event(e)
                {
                    return Err(SessionErr::Dead);
                }
                Ok(None)
            },
            // A stray response (stale id) — ignore.
            _ => Ok(None),
        }
    }

    /// One `tools/list`/`prompts/list` call, paginating on `nextCursor`.
    /// `None` when the server didn't advertise the capability.
    pub(super) fn list_all(&mut self, method: &str, offered: bool) -> SessionResult<Vec<Value>> {
        if !offered {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let id = self.next_id();
            let result = self.request(&wire::list_req(id, method, cursor.as_deref()), &mut |_| true)?;
            let key = method.strip_suffix("/list").unwrap_or(method);
            out.extend(result[key].as_array().into_iter().flatten().cloned());
            match result["nextCursor"].as_str() {
                Some(c) => cursor = Some(c.to_string()),
                None => return Ok(out),
            }
        }
    }

    /// Handshake, then list whatever the server advertised — the catalog
    /// `send` resolves the model against and `fetch` turns into models.
    pub(super) fn listings(&mut self) -> SessionResult<wire::Listings> {
        let id = self.next_id();
        let result = self.request(&wire::initialize_req(id), &mut |_| true)?;
        wire::send(&mut *self.stdin, &wire::initialized_note()).map_err(|_| SessionErr::Dead)?;
        let caps = &result["capabilities"];
        let info = &result["serverInfo"];
        let tools = self.list_all("tools/list", caps.get("tools").is_some())?;
        let prompts = self.list_all("prompts/list", caps.get("prompts").is_some())?;
        Ok(wire::Listings {
            server: info["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("mcp-server").to_string(),
            version: info["version"].as_str().unwrap_or_default().to_string(),
            tools,
            prompts,
        })
    }

    /// Run one resolved target to completion: `tools/call` opens a card
    /// and marks `call_ix` live for notifications; `prompts/get` streams
    /// message bubbles; `Server` replies with the server's info. Events
    /// flow through `on_event` — false ends the session `Dead`.
    pub(super) fn run(
        &mut self, target: &wire::Target, listings: &wire::Listings, on_event: &mut dyn FnMut(AgentEvent) -> bool,
    ) -> SessionResult<()> {
        const IX: usize = 0;
        let emit = |on_event: &mut dyn FnMut(AgentEvent) -> bool, events: Vec<AgentEvent>| -> SessionResult<()> {
            if events.into_iter().all(&mut *on_event) { Ok(()) } else { Err(SessionErr::Dead) }
        };
        match target {
            wire::Target::Tool { name, args } => {
                let start = AgentEvent::ToolCallStart {
                    ix: IX,
                    name: name.clone().into(),
                    detail: serde_json::to_string(args).unwrap_or_default().into(),
                };
                if !on_event(start) {
                    return Err(SessionErr::Dead);
                }
                self.call_ix = Some(IX);
                let id = self.next_id();
                let result = self.request(&wire::call_req(id, name, args), on_event);
                self.call_ix = None;
                emit(on_event, mcp_decode::call_result(&result?, IX))?;
            },
            wire::Target::Prompt { name, args } => {
                let start = AgentEvent::ToolCallStart {
                    ix: IX,
                    name: format!("prompt {name}").into(),
                    detail: serde_json::to_string(args).unwrap_or_default().into(),
                };
                if !on_event(start) {
                    return Err(SessionErr::Dead);
                }
                let id = self.next_id();
                let result = self.request(&wire::get_req(id, name, args), on_event)?;
                emit(on_event, mcp_decode::prompt_result(&result))?;
                emit(on_event, vec![AgentEvent::ToolCallEnd { ix: IX, ok: true }])?;
            },
            wire::Target::Server => {
                let text = mcp_decode::server_info_text(&listings.server, &listings.version);
                emit(on_event, vec![AgentEvent::TextDelta(text.into())])?;
            },
        }
        Ok(())
    }
}
