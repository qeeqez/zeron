//! Tests for the Ollama backend: NDJSON chat parsing, request-body shape,
//! `/api/tags` catalog parsing, and the live turn loop against a stub
//! daemon (an in-process `TcpListener` — no real Ollama, no spawned
//! processes). Declared from `backend/ollama.rs` via `#[path]` — `main.rs`
//! is at the SLOC cap.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::Receiver;

use serde_json::{Value, json};

use super::{OllamaBackend, chat_body, fetch_ollama_models, parse_chat_line, parse_tags};
use crate::backend::{AgentBackend, AgentEvent, TurnContext};

/// A one-shot stub daemon: accepts a single connection, captures the raw
/// request, replies `status` + `body`, closes. Returns the bound URL and
/// a channel that yields the captured request text.
fn stub_daemon(status: &str, body: &str) -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let (status, body) = (status.to_string(), body.to_string());
    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else { return };
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let trimmed = line.trim_end().to_string();
            if let Some(v) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            }
            if trimmed.is_empty() {
                break;
            }
            request.push_str(&trimmed);
            request.push('\n');
        }
        let mut buf = vec![0u8; content_length];
        if reader.read_exact(&mut buf).is_err() {
            return;
        }
        request.push_str(&String::from_utf8_lossy(&buf));
        let _ = tx.send(request);
        let resp = format!("{status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = reader.get_mut().write_all(resp.as_bytes());
    });
    (url, rx)
}

/// A local address nothing listens on: bind a port, drop the listener,
/// hand the freed port back. A connect to it is refused.
fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// Drain the reply stream until the channel closes.
fn events_of(stream: crate::backend::ReplyStream) -> Vec<AgentEvent> {
    stream.events.iter().collect()
}

#[test]
fn chat_line_streams_deltas_then_done() {
    let evs = parse_chat_line(r#"{"model":"m","message":{"role":"assistant","content":"Hel"},"done":false}"#);
    assert!(matches!(&evs[..], [AgentEvent::TextDelta(t)] if t == "Hel"));
    let evs = parse_chat_line(r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"total_duration":5}"#);
    assert!(matches!(&evs[..], [AgentEvent::Done]), "empty final chunk still ends the turn");
}

#[test]
fn chat_line_surfaces_errors_and_skips_junk() {
    let evs = parse_chat_line(r#"{"error":"model 'x' not found"}"#);
    assert!(matches!(&evs[..], [AgentEvent::Error(e)] if e.contains("model 'x' not found")));
    assert!(parse_chat_line("not json").is_empty());
    assert!(parse_chat_line(r#"{"done":false}"#).is_empty(), "keep-alive chunks emit nothing");
}

#[test]
fn chat_body_places_system_before_user() {
    let body = chat_body("llama3.2", "hi", Some("be terse"));
    assert_eq!(body["model"], json!("llama3.2"));
    assert_eq!(body["stream"], json!(true));
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0], json!({"role": "system", "content": "be terse"}));
    assert_eq!(msgs[1], json!({"role": "user", "content": "hi"}));
    // No instructions → no system message, and blank instructions don't
    // produce an empty one.
    let msgs = chat_body("m", "hi", None)["messages"].as_array().unwrap().clone();
    assert_eq!(msgs, vec![json!({"role": "user", "content": "hi"})]);
    let msgs = chat_body("m", "hi", Some("  "))["messages"].as_array().unwrap().clone();
    assert_eq!(msgs.len(), 1);
}

#[test]
fn tags_parse_into_model_info() {
    let body = json!({"models": [
        {"name": "llama3.2:latest", "model": "llama3.2:latest", "size": 1, "details": {"parameter_size": "3B", "family": "llama"}},
        {"name": "qwen3:8b", "model": "qwen3:8b", "details": {"parameter_size": "8B", "family": "qwen3"}},
        {"model": "no-name-field"}
    ]});
    let models = parse_tags(&body);
    assert_eq!(models.len(), 2, "entries without a name are dropped");
    assert_eq!(models[0].id.as_ref(), "llama3.2:latest");
    assert_eq!(models[0].description.as_ref(), "3B · llama");
    assert_eq!(models[1].id.as_ref(), "qwen3:8b");
    assert!(parse_tags(&json!({})).is_empty());
}

#[test]
fn send_streams_the_reply_and_posts_the_chat_shape() {
    let ndjson = concat!(
        r#"{"model":"m","message":{"role":"assistant","content":"Hel"},"done":false}"#,
        "\n",
        r#"{"model":"m","message":{"role":"assistant","content":"lo"},"done":false}"#,
        "\n",
        r#"{"model":"m","message":{"role":"assistant","content":""},"done":true}"#,
        "\n"
    );
    let (url, req) = stub_daemon("HTTP/1.1 200 OK", ndjson);
    let backend = OllamaBackend::new(url);
    let mut ctx = TurnContext::at(std::path::PathBuf::from("/tmp"), crate::backend::AccessMode::Auto);
    ctx.instructions = Some("be terse".into());
    let events = events_of(backend.send("hi", "llama3.2", "Agent", &ctx));
    assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], AgentEvent::TextDelta(t) if t == "Hel"));
    assert!(matches!(&events[1], AgentEvent::TextDelta(t) if t == "lo"));
    assert!(matches!(events[2], AgentEvent::Done));
    let raw = req.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(raw.starts_with("POST /api/chat"), "request line was: {}", raw.lines().next().unwrap_or(""));
    let body: Value = serde_json::from_str(&raw[raw.find('{').unwrap_or(0)..]).unwrap();
    assert_eq!(body["model"], json!("llama3.2"));
    assert_eq!(body["messages"][0]["role"], json!("system"));
    assert_eq!(body["messages"][1]["content"], json!("hi"));
}

#[test]
fn send_reports_connection_refused_as_error() {
    let backend = OllamaBackend::new(dead_url());
    let ctx = TurnContext::at(std::path::PathBuf::from("/tmp"), crate::backend::AccessMode::Auto);
    let events = events_of(backend.send("hi", "m", "Agent", &ctx));
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], AgentEvent::Error(e) if e.starts_with("ollama:")), "got {events:?}");
}

#[test]
fn send_reports_a_cut_stream_as_error() {
    // A reply that ends without `done: true` — the daemon died mid-turn.
    let (url, _req) = stub_daemon("HTTP/1.1 200 OK", "{\"message\":{\"content\":\"Hel\"},\"done\":false}\n");
    let backend = OllamaBackend::new(url);
    let ctx = TurnContext::at(std::path::PathBuf::from("/tmp"), crate::backend::AccessMode::Auto);
    let events = events_of(backend.send("hi", "m", "Agent", &ctx));
    assert!(matches!(&events[0], AgentEvent::TextDelta(t) if t == "Hel"));
    assert!(matches!(events.last(), Some(AgentEvent::Error(e)) if e.contains("stream ended")));
}

#[test]
fn fetch_lists_models_from_tags() {
    let (url, req) =
        stub_daemon("HTTP/1.1 200 OK", r#"{"models":[{"name":"llama3.2:latest","details":{"parameter_size":"3B","family":"llama"}}]}"#);
    let mut p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::Ollama, "Ollama".into());
    p.command = url;
    let models = fetch_ollama_models(&p).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.as_ref(), "llama3.2:latest");
    let raw = req.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(raw.starts_with("GET /api/tags"), "request line was: {}", raw.lines().next().unwrap_or(""));
}

#[test]
fn fetch_errors_when_the_daemon_is_down() {
    let mut p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::Ollama, "Ollama".into());
    p.command = dead_url();
    let err = fetch_ollama_models(&p).unwrap_err();
    assert!(err.starts_with("ollama:"), "got {err}");
}

#[test]
fn backend_declares_no_sessions_or_steer() {
    let backend = OllamaBackend::new(String::new());
    assert!(!backend.supports_steer());
    assert!(!backend.supports_sessions());
    assert!(backend.list_sessions().is_none());
    assert!(backend.resume_session("t1").is_none());
    assert!(backend.resume_command("t1").is_none());
    assert!(backend.models().is_empty(), "the catalog comes from /api/tags, not a static list");
}

#[test]
fn backend_for_builds_ollama() {
    let p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::Ollama, "Ollama".into());
    let b = crate::backend::backend_for(&p);
    assert_eq!(b.name(), "ollama");
}
