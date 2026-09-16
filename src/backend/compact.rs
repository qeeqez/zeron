//! Server-side context compaction: `thread/compact/start` over
//! `codex app-server`.
//!
//! Only chats bound to a persisted codex thread qualify — ephemeral
//! per-turn threads hold no history worth folding. The driver resumes the
//! thread on a fresh app-server, starts the compaction turn, and streams
//! its notifications back as `AgentEvent`s: a running "compact" tool card
//! while the server summarizes, the summary note plus the post-compaction
//! token usage when `thread/compacted` lands, and `Done` on
//! `turn/completed`.

use std::io::{BufRead, Write};

use serde_json::Value;

use super::appserver::TurnDecoder;
use super::rpc::{initialize_req, thread_compact_start_req, thread_resume_req};
use super::steer::{CodexSlot, kill_slot};
use super::{AgentEvent, ApprovalDecision, ApprovalRoute, ReplyStream, TurnContext};

/// Item key for the synthetic progress card — real items hash their own
/// ids, so a fixed key can't collide with a `contextCompaction` item.
fn compact_ix() -> usize {
    crate::backend_parse::item_ix(&serde_json::json!({"id": "__compact__"}))
}

/// Run `thread/compact/start` on the chat's bound codex thread. `None`
/// when the chat isn't bound — the caller falls back to a summarization
/// prompt turn.
pub(super) fn compact_thread(ctx: &TurnContext, env: &[(String, String)]) -> Option<ReplyStream> {
    let thread_id = ctx.thread_id.clone()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let slot = std::sync::Arc::new(CodexSlot::new());
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let env = env.to_vec();
    let thread_slot = slot.clone();
    std::thread::spawn(move || run_compact(&thread_id, &env, &thread_slot, &tx));
    Some(ReplyStream { events: rx, child: Some(slot), cancelled })
}

/// One compaction turn: spawn `codex app-server`, handshake, resume the
/// thread, start compaction, stream notifications until `turn/completed`
/// or EOF. No retry — a compaction that already ran can't be replayed
/// blindly, and a dead child surfaces as `Error`.
fn run_compact(thread_id: &str, env: &[(String, String)], slot: &CodexSlot, tx: &std::sync::mpsc::Sender<AgentEvent>) {
    let mut cmd = std::process::Command::new("codex");
    cmd.arg("app-server")
        .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    super::apply_env(&mut cmd, env);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error(format!("codex spawn: {e}").into()));
            return;
        },
    };
    let stdout = child.stdout.take().expect("piped");
    let mut stdin = child.stdin.take().expect("piped");
    let stderr = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut buf);
            buf.trim().to_string()
        })
    });
    *slot.child.lock() = Some(child);

    let result = drive_compact(&mut stdin, stdout, thread_id, tx);
    kill_slot(slot);
    if let Err(e) = result {
        let detail = stderr.and_then(|h| h.join().ok()).filter(|s| !s.is_empty());
        let msg = detail.map_or(e.clone(), |d| format!("{e}: {d}"));
        let _ = tx.send(AgentEvent::Error(msg.into()));
    }
}

/// Handshake → `thread/resume` → `thread/compact/start`, then decode the
/// compaction turn's notifications. Request ids: 1 = initialize,
/// 2 = resume, 3 = compact. Testable with in-memory cursors.
fn drive_compact(
    stdin: &mut dyn Write, stdout: impl std::io::Read, thread_id: &str, tx: &std::sync::mpsc::Sender<AgentEvent>,
) -> Result<(), String> {
    let send = |stdin: &mut dyn Write, v: &Value| -> Result<(), String> { writeln!(stdin, "{v}").map_err(|e| format!("codex stdin: {e}")) };
    send(stdin, &initialize_req(1))?;

    // Compaction never asks — deny any stray approval request outright.
    let mut decoder = TurnDecoder::new(ApprovalRoute::Auto(ApprovalDecision::Deny));
    let mut compacted = false;
    let mut errored = false;
    let reader = std::io::BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if msg.get("method").is_none() && msg.get("id").is_some() {
            if let Some(err) = msg.get("error") {
                let m = err["message"].as_str().unwrap_or("request failed");
                return Err(format!("codex: {m}"));
            }
            match msg["id"].as_i64() {
                Some(1) => {
                    send(stdin, &serde_json::json!({"method": "initialized", "params": {}}))?;
                    send(stdin, &thread_resume_req(2, thread_id, None))?;
                },
                Some(2) => {
                    send(stdin, &thread_compact_start_req(3, thread_id))?;
                    // The progress row opens as soon as the server takes
                    // the request — the compaction item itself carries no
                    // text, so the card is the transcript's only signal.
                    let _ = tx.send(AgentEvent::ToolCallStart {
                        ix: compact_ix(),
                        name: "compact".into(),
                        detail: "Summarizing thread context".into(),
                    });
                },
                _ => {},
            }
            continue;
        }
        // `thread/compacted` is the authoritative result: close the card
        // and land the summary note.
        if msg["method"].as_str() == Some("thread/compacted") {
            compacted = true;
            let _ = tx.send(AgentEvent::ToolCallEnd { ix: compact_ix(), ok: true });
            let _ = tx.send(AgentEvent::TextStart);
            let _ = tx.send(AgentEvent::TextDelta(
                "**Context compacted** — the thread's history was folded into a summary; the next turn resumes from it.".into(),
            ));
            continue;
        }
        let decoded = decoder.line(&line);
        errored |= decoded.events.iter().any(|e| matches!(e, AgentEvent::Error(_)));
        for e in decoded.events {
            if tx.send(e).is_err() {
                return Ok(());
            }
        }
        if let Some(resp) = decoded.response {
            send(stdin, &resp)?;
        }
        if decoded.turn_over {
            // Older servers may not emit `thread/compacted` — a clean
            // `turn/completed` still means the fold ran.
            if !compacted && !errored {
                let _ = tx.send(AgentEvent::ToolCallEnd { ix: compact_ix(), ok: true });
                let _ = tx.send(AgentEvent::TextStart);
                let _ = tx.send(AgentEvent::TextDelta("**Context compacted**.".into()));
            } else if errored {
                let _ = tx.send(AgentEvent::ToolCallEnd { ix: compact_ix(), ok: false });
            }
            return Ok(());
        }
    }
    if !compacted && !errored {
        return Err("codex closed stdout before the compaction turn completed".into());
    }
    Ok(())
}
