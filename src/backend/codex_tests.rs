//! Tests for the codex turn's sandbox/approval mapping and spawn command —
//! sibling file so `codex.rs` stays under the SLOC cap. No real subprocess
//! is ever spawned; `build_command` is inspected directly.

#[cfg(test)]
mod tests {
    use crate::backend::AccessMode;

    use crate::backend::codex::CodexTurn;
    use crate::backend::codex_turn::{Phase, advance_phase, approval_of, build_command, sandbox_of};

    fn turn(mode: &str, access: AccessMode) -> CodexTurn {
        CodexTurn::for_test(mode, access)
    }

    #[test]
    fn agent_mode_maps_access_to_sandbox() {
        let cases = [
            (AccessMode::Supervised, "read-only"),
            (AccessMode::AutoAcceptEdits, "workspace-write"),
            (AccessMode::Auto, "workspace-write"),
            (AccessMode::FullAccess, "danger-full-access"),
        ];
        for (access, want) in cases {
            assert_eq!(sandbox_of(&turn("Agent", access)), want);
        }
    }

    #[test]
    fn agent_mode_maps_access_to_approval() {
        let cases = [
            (AccessMode::Supervised, "on-request"),
            (AccessMode::AutoAcceptEdits, "on-failure"),
            (AccessMode::Auto, "never"),
            (AccessMode::FullAccess, "never"),
        ];
        for (access, want) in cases {
            assert_eq!(approval_of(&turn("Agent", access)), want);
        }
    }

    #[test]
    fn plan_and_ask_stay_read_only() {
        for mode in ["Plan", "Ask"] {
            for access in AccessMode::ALL {
                let t = turn(mode, access);
                assert_eq!(sandbox_of(&t), "read-only");
                assert_eq!(approval_of(&t), "never");
            }
        }
    }

    #[test]
    fn command_spawns_in_the_thread_workdir() {
        let cmd = build_command(&turn("Agent", AccessMode::Auto));
        assert_eq!(cmd.get_current_dir(), Some(std::path::Path::new("/tmp/thread-wt")));
    }

    #[test]
    fn instance_env_lands_on_the_spawned_command() {
        let mut t = turn("Agent", AccessMode::Auto);
        t.env = vec![
            ("CODEX_HOME".to_string(), "/tmp/codex-home".to_string()),
            (String::new(), "half-edited".to_string()),
        ];
        let envs: Vec<_> = build_command(&t).get_envs().map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string()))).collect();
        assert!(envs.contains(&(std::ffi::OsString::from("CODEX_HOME"), Some(std::ffi::OsString::from("/tmp/codex-home")))));
        assert!(!envs.iter().any(|(k, _)| k.is_empty()), "blank-key rows are skipped");
    }

    /// The initialize response drives the next handshake request — a turn
    /// bound to a session resumes its thread, a fresh turn starts one.
    fn handshake_writes(turn: &CodexTurn) -> String {
        let mut stdin = Vec::new();
        let mut phase = Phase::Init;
        let (tx, _rx) = std::sync::mpsc::channel();
        let init_reply = serde_json::json!({"id": 1, "result": {}});
        advance_phase(&mut phase, turn, &init_reply, &mut stdin, &tx).unwrap();
        String::from_utf8(stdin).unwrap()
    }

    #[test]
    fn bound_turn_resumes_its_thread() {
        let sent = handshake_writes(&turn("Agent", AccessMode::Auto).resuming("tid-7"));
        assert!(sent.contains(r#""method":"initialized""#), "stdin was: {sent}");
        assert!(sent.contains(r#""method":"thread/resume""#), "stdin was: {sent}");
        assert!(sent.contains(r#""threadId":"tid-7""#), "stdin was: {sent}");
        // The chat's model and access ride along as overrides.
        assert!(sent.contains(r#""model":"gpt-5""#), "stdin was: {sent}");
        assert!(sent.contains(r#""sandbox":"workspace-write""#), "stdin was: {sent}");
        assert!(sent.contains(r#""approvalPolicy":"never""#), "stdin was: {sent}");
        assert!(!sent.contains("thread/start"), "stdin was: {sent}");
    }

    #[test]
    fn unbound_turn_still_starts_a_thread() {
        let sent = handshake_writes(&turn("Agent", AccessMode::Auto));
        assert!(sent.contains(r#""method":"thread/start""#), "stdin was: {sent}");
        assert!(!sent.contains("thread/resume"), "stdin was: {sent}");
    }

    /// Drive the handshake through `Phase::Thread` and return everything
    /// written to stdin — the turn/start request is the second write.
    fn turn_start_wire(turn: &CodexTurn) -> String {
        let mut stdin = Vec::new();
        let mut phase = Phase::Init;
        let (tx, _rx) = std::sync::mpsc::channel();
        advance_phase(&mut phase, turn, &serde_json::json!({"id": 1, "result": {}}), &mut stdin, &tx).unwrap();
        advance_phase(&mut phase, turn, &serde_json::json!({"id": 2, "result": {"thread": {"id": "tid-9"}}}), &mut stdin, &tx).unwrap();
        String::from_utf8(stdin).unwrap()
    }

    #[test]
    fn thread_reply_binds_the_chat_to_the_thread() {
        // The handshake's thread id reaches the chat as ThreadBound so
        // later sends — in-session and after a relaunch — resume it.
        let mut stdin = Vec::new();
        let mut phase = Phase::Init;
        let (tx, rx) = std::sync::mpsc::channel();
        let t = turn("Agent", AccessMode::Auto);
        advance_phase(&mut phase, &t, &serde_json::json!({"id": 1, "result": {}}), &mut stdin, &tx).unwrap();
        advance_phase(&mut phase, &t, &serde_json::json!({"id": 2, "result": {"thread": {"id": "tid-9"}}}), &mut stdin, &tx).unwrap();
        let bound: Vec<_> = rx.try_iter().collect();
        assert!(bound.iter().any(|e| matches!(e, crate::backend::AgentEvent::ThreadBound(id) if id == "tid-9")), "events were: {bound:?}");
    }

    #[test]
    fn resume_reply_rebinds_the_same_thread() {
        let mut stdin = Vec::new();
        let mut phase = Phase::Init;
        let (tx, rx) = std::sync::mpsc::channel();
        let t = turn("Agent", AccessMode::Auto).resuming("tid-7");
        advance_phase(&mut phase, &t, &serde_json::json!({"id": 1, "result": {}}), &mut stdin, &tx).unwrap();
        advance_phase(&mut phase, &t, &serde_json::json!({"id": 2, "result": {"thread": {"id": "tid-7"}}}), &mut stdin, &tx).unwrap();
        let bound: Vec<_> = rx.try_iter().collect();
        assert!(bound.iter().any(|e| matches!(e, crate::backend::AgentEvent::ThreadBound(id) if id == "tid-7")), "events were: {bound:?}");
    }

    #[test]
    fn turn_start_carries_the_selected_effort() {
        let mut t = turn("Agent", AccessMode::Auto);
        t.effort = Some("high".into());
        let sent = turn_start_wire(&t);
        assert!(sent.contains(r#""method":"turn/start""#), "stdin was: {sent}");
        assert!(sent.contains(r#""effort":"high""#), "stdin was: {sent}");
    }

    #[test]
    fn turn_start_omits_effort_when_unset() {
        let sent = turn_start_wire(&turn("Agent", AccessMode::Auto));
        assert!(sent.contains(r#""method":"turn/start""#), "stdin was: {sent}");
        assert!(!sent.contains("effort"), "stdin was: {sent}");
    }

    #[test]
    fn turn_start_carries_image_attachments() {
        let mut t = turn("Agent", AccessMode::Auto);
        t.images = vec![std::path::PathBuf::from("/tmp/shot.png")];
        let sent = turn_start_wire(&t);
        let start = sent.lines().find(|l| l.contains("turn/start")).expect("stdin was: {sent}");
        let req: serde_json::Value = serde_json::from_str(start).unwrap();
        assert_eq!(req["params"]["input"][0]["type"], serde_json::json!("text"));
        assert_eq!(req["params"]["input"][1], serde_json::json!({"type": "localImage", "path": "/tmp/shot.png"}));
    }

    #[test]
    fn thread_start_carries_developer_instructions() {
        let mut t = turn("Agent", AccessMode::Auto);
        t.instructions = Some("be terse".into());
        let sent = handshake_writes(&t);
        let start = sent.lines().find(|l| l.contains("thread/start")).expect("stdin was: {sent}");
        let req: serde_json::Value = serde_json::from_str(start).unwrap();
        assert_eq!(req["params"]["developerInstructions"], serde_json::json!("be terse"));
        // The base instructions stay the server's — we never replace them.
        assert!(req["params"].get("baseInstructions").is_none());
    }

    #[test]
    fn thread_start_omits_instructions_when_unset() {
        let sent = handshake_writes(&turn("Agent", AccessMode::Auto));
        let start = sent.lines().find(|l| l.contains("thread/start")).expect("stdin was: {sent}");
        let req: serde_json::Value = serde_json::from_str(start).unwrap();
        assert!(req["params"].get("developerInstructions").is_none());
    }

    #[test]
    fn thread_resume_reasserts_instructions() {
        let mut t = turn("Agent", AccessMode::Auto).resuming("tid-7");
        t.instructions = Some("be terse".into());
        let sent = handshake_writes(&t);
        let resume = sent.lines().find(|l| l.contains("thread/resume")).expect("stdin was: {sent}");
        let req: serde_json::Value = serde_json::from_str(resume).unwrap();
        assert_eq!(req["params"]["developerInstructions"], serde_json::json!("be terse"));
    }

    #[test]
    fn compact_needs_a_bound_thread() {
        use crate::backend::{AgentBackend, TurnContext};
        // No thread id → no stream: the caller falls back to a prompt turn.
        let ctx = TurnContext::at(std::path::PathBuf::from("/tmp"), crate::backend::AccessMode::Auto);
        assert!(crate::backend::CodexCliBackend::new(Vec::new()).compact(&ctx).is_none());
    }
}

// `turn/steer` tests live in `codex_steer_tests.rs`, `account/*` auth
// tests in `codex_auth_tests.rs` — this file is at the SLOC cap.
