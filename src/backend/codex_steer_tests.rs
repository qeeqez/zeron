//! Tests for codex `turn/steer`: the handshake-driven turn handle, the
//! wire request, and steer-reply handling — sibling file so
//! `codex_tests.rs` stays under the SLOC cap.

#[cfg(test)]
mod steer_tests {
    use crate::backend::AccessMode;
    use crate::backend::codex::CodexTurn;
    use crate::backend::codex_turn::{Phase, advance_phase};

    fn turn(mode: &str, access: AccessMode) -> CodexTurn {
        CodexTurn::for_test(mode, access)
    }

    /// Drive the handshake to `Phase::Run` over a captured stdin, then
    /// return the buffer the turn's steer writes into.
    fn running_turn(mode: &str) -> (CodexTurn, std::sync::Arc<parking_lot::Mutex<Vec<u8>>>) {
        let mut turn = turn(mode, AccessMode::Auto);
        let buf = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        turn.slot = std::sync::Arc::new(crate::backend::steer::CodexSlot::for_test(buf.clone()));
        let mut stdin = turn.slot.stdin.clone();
        let mut phase = Phase::Init;
        let (tx, _rx) = std::sync::mpsc::channel();
        advance_phase(&mut phase, &turn, &serde_json::json!({"id": 1, "result": {}}), &mut stdin, &tx).unwrap();
        advance_phase(&mut phase, &turn, &serde_json::json!({"id": 2, "result": {"thread": {"id": "tid-9"}}}), &mut stdin, &tx).unwrap();
        advance_phase(&mut phase, &turn, &serde_json::json!({"id": 3, "result": {"turn": {"id": "u-4"}}}), &mut stdin, &tx).unwrap();
        (turn, buf)
    }

    #[test]
    fn steer_writes_turn_steer_request() {
        use crate::backend::TurnHandle;
        let (turn, buf) = running_turn("Agent");
        assert!(turn.slot.steer("keep going"));
        let sent = String::from_utf8(buf.lock().clone()).unwrap();
        let line = sent.lines().last().unwrap();
        let msg: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(msg["method"], "turn/steer");
        assert_eq!(msg["params"]["threadId"], "tid-9");
        assert_eq!(msg["params"]["expectedTurnId"], "u-4");
        assert_eq!(msg["params"]["input"][0]["type"], "text");
        assert_eq!(msg["params"]["input"][0]["text"], "keep going");
        assert!(msg["id"].as_i64().unwrap() >= 4, "steer ids must not collide with the handshake");
    }

    #[test]
    fn steer_needs_the_turn_id() {
        use crate::backend::TurnHandle;
        // Handshake hasn't reached turn/start — no ids, steer declines.
        let turn = turn("Agent", AccessMode::Auto);
        assert!(!turn.slot.steer("too early"));
        // Thread id alone isn't enough.
        turn.slot.ids.lock().0 = Some("tid-9".into());
        assert!(!turn.slot.steer("still early"));
    }

    #[test]
    fn steer_fails_when_stdin_closed() {
        use crate::backend::TurnHandle;
        let (turn, _buf) = running_turn("Agent");
        *turn.slot.stdin.0.lock() = None; // turn ended, pipe dropped
        assert!(!turn.slot.steer("too late"));
    }

    #[test]
    fn steer_error_reply_does_not_kill_the_turn() {
        let (turn, _buf) = running_turn("Agent");
        let mut stdin = turn.slot.stdin.clone();
        let mut phase = Phase::Run;
        let (tx, _rx) = std::sync::mpsc::channel();
        // A rejected steer (stale expectedTurnId) is consumed, not fatal.
        let rejected = serde_json::json!({"id": 4, "error": {"message": "turn mismatch"}});
        assert!(advance_phase(&mut phase, &turn, &rejected, &mut stdin, &tx).unwrap());
        // A steer success reply is consumed too.
        let ok = serde_json::json!({"id": 5, "result": {"turnId": "u-4"}});
        assert!(!advance_phase(&mut phase, &turn, &ok, &mut stdin, &tx).unwrap());
        // Handshake errors still fail.
        let mut phase = Phase::Init;
        let bad = serde_json::json!({"id": 1, "error": {"message": "boom"}});
        assert!(advance_phase(&mut phase, &turn, &bad, &mut stdin, &tx).is_err());
    }

    #[test]
    fn reply_stream_steer_routes_to_stdin() {
        let (turn, buf) = running_turn("Agent");
        let (_tx, events) = std::sync::mpsc::channel();
        let stream = crate::backend::ReplyStream {
            events,
            child: Some(turn.slot.clone()),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        assert!(stream.steer("mid-turn note"));
        let sent = String::from_utf8(buf.lock().clone()).unwrap();
        assert!(sent.contains(r#""method":"turn/steer""#), "stdin was: {sent}");
        assert!(sent.contains("mid-turn note"), "stdin was: {sent}");
        // A stream without a steerable handle declines.
        let (_tx, events) = std::sync::mpsc::channel();
        let bare = crate::backend::ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        assert!(!bare.steer("nope"));
    }

    #[test]
    fn codex_declares_steer_support() {
        use crate::backend::AgentBackend;
        assert!(crate::backend::CodexCliBackend::new(Vec::new()).supports_steer());
        assert!(!crate::backend::SimBackend.supports_steer());
    }
}
