//! HTTP transport: POST the prompt, stream NDJSON events back.

use super::{AgentBackend, AgentEvent, ReplyStream};

/// Backend that POSTs the prompt to an HTTP endpoint and reads an NDJSON
/// event stream back — same line format as `codex exec --json`, so the
/// parser is shared. Auth is a bearer token read from an env var at send
/// time; the key is never persisted.
pub struct HttpBackend {
    url: String,
    /// Env var holding the token; empty means no Authorization header.
    key_env: String,
    /// The instance's Variables — consulted for `key_env` before the
    /// process environment, so an API key can live on the instance.
    env: Vec<(String, String)>,
}

impl HttpBackend {
    pub fn new(url: String, key_env: String, env: Vec<(String, String)>) -> Self {
        Self { url, key_env, env }
    }
}

impl AgentBackend for HttpBackend {
    fn name(&self) -> &'static str {
        "http"
    }

    fn send(&self, prompt: &str, model: &str, mode: &str, ctx: &super::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let turn = std::sync::Arc::new(HttpTurn {
            url: self.url.clone(),
            key_env: self.key_env.clone(),
            env: self.env.clone(),
            prompt: crate::instructions::prefixed(prompt, ctx.instructions.as_deref()),
            model: model.to_string(),
            mode: mode.to_string(),
            cancelled: cancelled.clone(),
        });
        std::thread::spawn(move || run_http(&turn, &tx));
        ReplyStream { events: rx, child: None, cancelled }
    }
}

/// Everything one HTTP turn needs — bundled so the runner stays under
/// the argument-count lint.
struct HttpTurn {
    url: String,
    key_env: String,
    /// The instance's Variables — `key_env` resolves here first.
    env: Vec<(String, String)>,
    prompt: String,
    model: String,
    mode: String,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// One HTTP turn: POST the prompt, stream NDJSON lines until Done/EOF.
/// No retry — a streamed response can't be replayed safely once events
/// have been emitted, and pre-response failures surface as `Error`.
fn run_http(turn: &HttpTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    use std::io::BufRead;
    let body = request_body(turn);
    let mut req = ureq::post(&turn.url).config().timeout_global(Some(std::time::Duration::from_secs(300))).build();
    // The instance's Variables win over the process env — that's how a
    // per-provider API key works without touching the shell env.
    let key = turn.env.iter().find(|(k, _)| k.trim() == turn.key_env).map(|(_, v)| v.as_str());
    if !turn.key_env.is_empty()
        && let Some(key) = key.map(str::to_string).or_else(|| std::env::var(&turn.key_env).ok())
        && !key.is_empty()
    {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = match req.send_json(&body) {
        Ok(r) => r,
        Err(e) => {
            tx.send(AgentEvent::Error(format!("http: {e}").into())).ok();
            return;
        },
    };
    // `lines()` blocks until data arrives, so a quiet server would ignore
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
    loop {
        if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match line_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) if !emit_line(&line, tx) => return,
            Ok(_) => {},
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The POST body for one turn — `prompt` already carries any instructions
/// prefix (`send` merges them in, since the endpoint has no system field).
fn request_body(turn: &HttpTurn) -> serde_json::Value {
    serde_json::json!({ "prompt": turn.prompt, "model": turn.model, "mode": turn.mode })
}

/// Parse one NDJSON line and forward its events. Returns false when the
/// receiver is gone — the caller should stop the turn.
fn emit_line(line: &str, tx: &std::sync::mpsc::Sender<AgentEvent>) -> bool {
    crate::backend_parse::parse_codex_line(line).iter().all(|e| tx.send(e.clone()).is_ok())
}

#[cfg(test)]
mod tests {
    use super::{HttpTurn, request_body};

    fn turn(prompt: &str) -> HttpTurn {
        HttpTurn {
            url: "http://x".into(),
            key_env: String::new(),
            env: Vec::new(),
            prompt: prompt.into(),
            model: "m".into(),
            mode: "Agent".into(),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn body_carries_the_prefixed_prompt() {
        // `send` merges instructions into the prompt via
        // `instructions::prefixed` — the body ships it verbatim.
        let prompt = crate::instructions::prefixed("do it", Some("be terse"));
        let body = request_body(&turn(&prompt));
        assert_eq!(body["prompt"], serde_json::json!(prompt));
        assert!(body["prompt"].as_str().unwrap().starts_with("<system_instructions>"));
    }
}
