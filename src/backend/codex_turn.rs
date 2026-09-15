//! One `codex app-server` turn: spawn, handshake, stream notifications
//! until `turn/completed` or EOF, reap, classify. Split from `codex.rs` so
//! both stay under the SLOC cap.

use serde_json::Value;
use std::io::BufRead;

use super::AgentEvent;
use super::appserver::TurnDecoder;
use super::codex::CodexTurn;
use super::rpc::{initialize_req, thread_resume_req, thread_start_req, turn_start_req};
use super::steer::kill_slot;

pub(super) enum CodexOutcome {
    Done,
    Cancelled,
    Dead,
    Failed(String),
}

/// Handshake phase: which request id we're waiting on next.
pub(super) enum Phase {
    Init,
    Thread,
    Turn,
    Run,
}

/// The `codex app-server` command for one turn — extracted so tests can
/// assert the spawn cwd without launching a real process.
pub(super) fn build_command(turn: &CodexTurn) -> std::process::Command {
    let mut cmd = std::process::Command::new("codex");
    cmd.arg("app-server")
        .current_dir(&turn.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    super::apply_env(&mut cmd, &turn.env);
    cmd
}

pub(super) fn spawn_codex(turn: &CodexTurn, tx: &std::sync::mpsc::Sender<AgentEvent>) -> (CodexOutcome, bool) {
    let mut cmd = build_command(turn);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (CodexOutcome::Failed(format!("codex spawn: {e}")), false),
    };
    let stdout = child.stdout.take().expect("piped");
    let stdin = child.stdin.take().expect("piped");
    // Drain stderr on a thread from spawn — a chatty child blocks on a
    // full pipe before stdout EOF, and we want the text on failure.
    let stderr = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut buf);
            buf.trim().to_string()
        })
    });
    // Publish stdin before the child: a steer between the two stores would
    // write to a process the slot can't kill yet.
    *turn.slot.stdin.0.lock() = Some(Box::new(stdin));
    *turn.slot.child.lock() = Some(child);

    let mut stdin = turn.slot.stdin.clone();
    if let Err(e) = stdin.write_line(&initialize_req(1)) {
        kill_slot(&turn.slot);
        return (CodexOutcome::Failed(e), false);
    }

    let reader = std::io::BufReader::new(stdout);
    let mut decoder = TurnDecoder::new(approval_route_of(turn));
    let mut phase = Phase::Init;
    let (mut done, mut emitted) = (false, false);
    for line in reader.lines().map_while(Result::ok) {
        // Responses to our own requests advance the phase machine — and
        // steer replies (ids ≥ 4) are consumed silently here.
        if let Ok(msg) = serde_json::from_str::<Value>(&line)
            && msg.get("method").is_none()
            && msg.get("id").is_some()
        {
            match advance_phase(&mut phase, turn, &msg, &mut stdin) {
                Ok(true) => {},
                Ok(false) => continue,
                Err(e) => {
                    kill_slot(&turn.slot);
                    return (CodexOutcome::Failed(e), emitted);
                },
            }
            continue;
        }
        let decoded = decoder.line(&line);
        done |= decoded.turn_over;
        for e in decoded.events {
            emitted = true;
            if tx.send(e).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
        if let Some(pending) = decoded.pending {
            // The approval card is on screen — block until the user
            // answers (or the responder drops on cancel → Deny), then
            // write the decision back to the server.
            if pending.answer(&mut stdin).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
        if let Some(resp) = decoded.response {
            // Other server requests (elicitation, user input): answer
            // immediately so the turn can't hang waiting on us.
            if stdin.write_line(&resp).is_err() {
                kill_slot(&turn.slot);
                return (CodexOutcome::Dead, emitted);
            }
        }
        if done {
            break;
        }
    }
    // The turn is over — close stdin so a late steer fails fast instead of
    // writing into a pipe the server already dropped.
    *turn.slot.stdin.0.lock() = None;
    if done {
        return (CodexOutcome::Done, emitted);
    }
    // EOF without turn/completed: killed by cancel() or crashed.
    let Some(mut child) = turn.slot.child.lock().take() else { return (CodexOutcome::Cancelled, emitted) };
    let outcome = match child.wait() {
        Ok(s) if s.success() => CodexOutcome::Failed("codex exited without completing".into()),
        Ok(s) => {
            let detail = stderr
                .and_then(|h| h.join().ok())
                .filter(|e| !e.is_empty())
                .map(|e| format!(": {e}"))
                .unwrap_or_default();
            CodexOutcome::Failed(format!("codex exited with {s}{detail}"))
        },
        Err(e) => CodexOutcome::Failed(format!("codex wait: {e}")),
    };
    (outcome, emitted)
}

/// Handle a response to one of our handshake requests: send the next
/// request in the sequence and record the thread/turn ids `turn/steer`
/// needs. Returns Ok(true) when the line was consumed.
pub(super) fn advance_phase(phase: &mut Phase, turn: &CodexTurn, msg: &Value, stdin: &mut dyn std::io::Write) -> Result<bool, String> {
    let id = msg["id"].as_i64().unwrap_or(-1);
    if let Some(err) = msg.get("error") {
        // Steer replies (ids ≥ 4) are advisory: a rejected steer must not
        // kill the turn it tried to join.
        if id >= 4 {
            return Ok(true);
        }
        let m = err["message"].as_str().unwrap_or("request failed");
        return Err(format!("codex: {m}"));
    }
    match (std::mem::replace(phase, Phase::Run), id) {
        (Phase::Init, 1) => {
            // `initialized` notification, then open the turn's thread —
            // resume a bound session's thread, else start an ephemeral one.
            let open = match &turn.resume {
                Some(tid) => thread_resume_req(2, tid, Some(&turn.model), Some(sandbox_of(turn)), Some(approval_of(turn))),
                None => thread_start_req(2, &turn.model, sandbox_of(turn), approval_of(turn), &turn.cwd),
            };
            writeln!(stdin, "{}", serde_json::json!({"method": "initialized", "params": {}}))
                .and_then(|()| writeln!(stdin, "{open}"))
                .map_err(|e| format!("codex stdin: {e}"))?;
            *phase = Phase::Thread;
            Ok(true)
        },
        (Phase::Thread, 2) => {
            let tid = msg["result"]["thread"]["id"].as_str().ok_or("codex: no thread id")?.to_string();
            turn.slot.ids.lock().0 = Some(tid.clone());
            writeln!(stdin, "{}", turn_start_req(3, &tid, &turn.prompt, turn.effort.as_deref(), &turn.images))
                .map_err(|e| format!("codex stdin: {e}"))?;
            *phase = Phase::Turn;
            Ok(true)
        },
        (Phase::Turn, 3) => {
            let turn_id = msg["result"]["turn"]["id"].as_str().ok_or("codex: no turn id")?.to_string();
            turn.slot.ids.lock().1 = Some(turn_id);
            *phase = Phase::Run;
            Ok(true)
        },
        // Steer reply or a stray response — restore the phase and consume
        // the line so the decoder never sees it.
        (old, _) => {
            *phase = old;
            Ok(false)
        },
    }
}

/// Sandbox for `thread/start` — Agent honors the access setting, Plan/Ask
/// stay read-only, mirroring the old `codex exec -s` mapping.
pub(super) fn sandbox_of(turn: &CodexTurn) -> &'static str {
    if turn.mode == "Agent" { turn.access.sandbox_arg() } else { "read-only" }
}

/// `approvalPolicy` for `thread/start` — Agent honors the access setting's
/// ask/auto split; Plan/Ask never prompt (their sandbox is read-only).
pub(super) fn approval_of(turn: &CodexTurn) -> &'static str {
    if turn.mode == "Agent" { turn.access.approval_arg() } else { "never" }
}

/// How this turn answers approval requests: Agent mode honors the access
/// setting's ask/auto split; Plan/Ask deny outright — their read-only
/// sandbox must never see an approval it could grant.
pub(super) fn approval_route_of(turn: &CodexTurn) -> super::ApprovalRoute {
    if turn.mode == "Agent" {
        turn.access.approval_route()
    } else {
        super::ApprovalRoute::Auto(super::ApprovalDecision::Deny)
    }
}
