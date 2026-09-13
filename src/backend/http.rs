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
}

impl HttpBackend {
    pub fn new(url: String, key_env: String) -> Self {
        Self { url, key_env }
    }
}

impl AgentBackend for HttpBackend {
    fn name(&self) -> &'static str {
        "http"
    }

    fn send(&self, prompt: &str, model: &str, mode: &str) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let turn = std::sync::Arc::new(HttpTurn {
            url: self.url.clone(),
            key_env: self.key_env.clone(),
            prompt: prompt.to_string(),
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
    let body = serde_json::json!({ "prompt": turn.prompt, "model": turn.model, "mode": turn.mode });
    let mut req = ureq::post(&turn.url).config().timeout_global(Some(std::time::Duration::from_secs(300))).build();
    if !turn.key_env.is_empty()
        && let Ok(key) = std::env::var(&turn.key_env)
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
    let reader = std::io::BufReader::new(resp.into_body().into_reader());
    for line in reader.lines().map_while(Result::ok) {
        if turn.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        for e in crate::backend_parse::parse_codex_line(&line) {
            if tx.send(e).is_err() {
                return;
            }
        }
    }
}
