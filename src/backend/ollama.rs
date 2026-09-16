//! Ollama transport: POST `/api/chat`, stream NDJSON deltas back.
//!
//! A local daemon speaks plain HTTP with no auth and keeps no threads —
//! every turn is a self-contained request, so `resume_session`/
//! `resume_command`/`list_sessions` stay at their `None` defaults and
//! `supports_steer`/`supports_sessions` stay false. Mode and access don't
//! reach the wire either: the endpoint has no tools, so a turn is a plain
//! chat completion whatever the composer mode says.

use serde_json::Value;

use super::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::model::ModelInfo;
use crate::providers::ProviderInstance;

/// Backend that chats with a local Ollama daemon. `base_url` is the
/// instance's `command` field — the daemon root (`http://localhost:11434`
/// by default), with `/api/chat` and `/api/tags` under it.
pub struct OllamaBackend {
    base_url: String,
}

impl OllamaBackend {
    /// The daemon root a fresh instance points at — also the fallback when
    /// the field is left blank.
    pub const DEFAULT_URL: &'static str = "http://localhost:11434";

    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }

    /// The configured root with whitespace and trailing slashes stripped —
    /// endpoint paths append cleanly. Blank falls back to the default.
    fn base(&self) -> &str {
        let trimmed = self.base_url.trim().trim_end_matches('/');
        if trimmed.is_empty() { Self::DEFAULT_URL } else { trimmed }
    }
}

impl AgentBackend for OllamaBackend {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn send(&self, prompt: &str, model: &str, _mode: &str, ctx: &TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let turn = std::sync::Arc::new(OllamaTurn {
            url: format!("{}/api/chat", self.base()),
            body: chat_body(model, prompt, ctx.instructions.as_deref()),
            cancelled: cancelled.clone(),
        });
        std::thread::spawn(move || run_ollama(&turn, &tx));
        ReplyStream { events: rx, child: None, cancelled }
    }
}

/// Everything one Ollama turn needs — bundled so the runner stays under
/// the argument-count lint.
struct OllamaTurn {
    /// `POST` target — `<base>/api/chat`.
    url: String,
    /// Serialized request body (`model`, `messages`, `stream: true`).
    body: Value,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The `/api/chat` body: merged instructions ride as a leading `system`
/// message (Ollama's dedicated channel — no `<system_instructions>`
/// prefix), the prompt is the single `user` message. The backend contract
/// hands each turn only its own prompt, so the array never carries prior
/// turns.
fn chat_body(model: &str, prompt: &str, instructions: Option<&str>) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = instructions.map(str::trim).filter(|i| !i.is_empty()) {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    messages.push(serde_json::json!({"role": "user", "content": prompt}));
    serde_json::json!({"model": model, "messages": messages, "stream": true})
}

/// One Ollama turn: POST the chat request, stream NDJSON lines until
/// `done` or EOF. No retry — a streamed response can't be replayed safely
/// once deltas have been emitted, and pre-response failures surface as
/// `Error`.
fn run_ollama(turn: &OllamaTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    use std::io::BufRead;
    // Local daemons can take a while to load a model before the first
    // token — the same 300s cap the http backend uses.
    let req = ureq::post(&turn.url).config().timeout_global(Some(std::time::Duration::from_secs(300))).build();
    let resp = match req.send_json(&turn.body) {
        Ok(r) => r,
        Err(e) => {
            tx.send(AgentEvent::Error(format!("ollama: {e}").into())).ok();
            return;
        },
    };
    // `lines()` blocks until data arrives, so a quiet daemon would ignore
    // cancellation until the global timeout. Bridge lines through a
    // channel and poll `cancelled` between recv windows instead — the
    // reader thread exits on its own once the request finishes or times
    // out (300s cap above).
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(resp.into_body().into_reader());
        for line in reader.lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut done = false;
    loop {
        if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match line_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) => {
                if !pump_line(&line, tx, &mut done) {
                    return;
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if !done {
        // The daemon always ends a turn with `done: true` — EOF before it
        // means the stream was cut mid-reply.
        tx.send(AgentEvent::Error("ollama: stream ended before the reply completed".into())).ok();
    }
}

/// Parse one NDJSON line and forward its events; `done` records the
/// terminal chunk. Returns false when the receiver is gone or the daemon
/// finished the turn — the caller should stop the turn.
fn pump_line(line: &str, tx: &std::sync::mpsc::Sender<AgentEvent>, done: &mut bool) -> bool {
    let mut alive = true;
    for e in parse_chat_line(line) {
        *done |= matches!(e, AgentEvent::Done);
        alive &= tx.send(e).is_ok();
    }
    alive && !*done
}

/// Map one `/api/chat` NDJSON line to `AgentEvent`s: `message.content`
/// chunks become `TextDelta`, `done: true` becomes `Done`, a top-level
/// `error` becomes `Error`. Unparseable lines and keep-alive chunks with
/// empty content emit nothing.
fn parse_chat_line(line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    if let Some(err) = v["error"].as_str() {
        return vec![AgentEvent::Error(format!("ollama: {err}").into())];
    }
    let mut out = Vec::new();
    if let Some(delta) = v["message"]["content"].as_str().filter(|c| !c.is_empty()) {
        out.push(AgentEvent::TextDelta(delta.into()));
    }
    if v["done"].as_bool() == Some(true) {
        out.push(AgentEvent::Done);
    }
    out
}

/// Fetch the daemon's installed models (`GET <base>/api/tags`) — the
/// instance's catalog refresh. `Err` on connection failure, non-2xx, or a
/// malformed body; callers fall back to the cached catalog.
pub fn fetch_ollama_models(p: &ProviderInstance) -> Result<Vec<ModelInfo>, String> {
    let base = OllamaBackend::new(p.command.clone());
    let mut resp = ureq::get(format!("{}/api/tags", base.base()))
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .call()
        .map_err(|e| format!("ollama: {e}"))?;
    let body: Value = resp.body_mut().read_json().map_err(|e| format!("ollama: {e}"))?;
    Ok(parse_tags(&body))
}

/// `/api/tags` body → catalog entries: `name` is the id the chat request
/// takes, `details.parameter_size`/`family` make the one-line description.
fn parse_tags(body: &Value) -> Vec<ModelInfo> {
    body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m["name"].as_str()?;
            let description = [m["details"]["parameter_size"].as_str(), m["details"]["family"].as_str()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            Some(ModelInfo {
                id: name.into(),
                label: name.into(),
                description: description.into(),
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "../ollama_tests.rs"]
mod ollama_tests;
