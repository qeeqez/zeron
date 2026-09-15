//! Tests for the `claude -p --output-format stream-json` decoder — pure
//! parsing, no subprocess. Line shapes mirror a captured stream with
//! `--include-partial-messages`.

use super::claude_parse::ClaudeDecoder;
use crate::backend::{AgentBackend, AgentEvent};

fn events(d: &mut ClaudeDecoder, line: &str) -> Vec<AgentEvent> {
    d.line(line).events
}

#[test]
fn text_streams_deltas_then_snapshot_skips() {
    let mut d = ClaudeDecoder::new();
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":1}}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::Usage { input: 12, output: 1 }));

    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
    );
    assert!(matches!(evs[0], AgentEvent::TextStart));

    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "hello"));
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == " world"));

    // The assistant snapshot repeats the full text — must not re-emit.
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello world"}],"usage":{"input_tokens":12,"output_tokens":5}}}"#,
    );
    assert!(evs.iter().all(|e| matches!(e, AgentEvent::Usage { .. })));
}

#[test]
fn assistant_snapshot_without_partials_emits_text() {
    let mut d = ClaudeDecoder::new();
    let evs = events(&mut d, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"full reply"}]}}"#);
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "full reply"));
}

#[test]
fn tool_use_opens_card_and_result_closes_it() {
    let mut d = ClaudeDecoder::new();
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}}}"#,
    );
    let ix = match &evs[0] {
        AgentEvent::ToolCallStart { ix, name, .. } => {
            assert_eq!(name.as_ref(), "Bash");
            *ix
        },
        e => panic!("expected ToolCallStart, got {e:?}"),
    };

    // Snapshot carries the real input — streamed card gets the command
    // summary as its first output line, not a second card.
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { ix: i, output } if *i == ix && output == "cargo test\n"));

    let evs = events(
        &mut d,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"ok 3 passed"}]}]}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { ix: i, .. } if *i == ix));
    assert!(matches!(&evs[1], AgentEvent::ToolCallEnd { ix: i, ok: true } if *i == ix));
}

#[test]
fn tool_use_snapshot_only_and_error_result() {
    let mut d = ClaudeDecoder::new();
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Read","input":{"file_path":"/tmp/x.rs"}}]}}"#,
    );
    let ix = match &evs[0] {
        AgentEvent::ToolCallStart { ix, name, detail } => {
            assert_eq!(name.as_ref(), "Read");
            assert_eq!(detail.as_ref(), "/tmp/x.rs");
            *ix
        },
        e => panic!("expected ToolCallStart, got {e:?}"),
    };

    let evs = events(
        &mut d,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_2","is_error":true,"content":"permission denied"}]}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { ix: i, output } if *i == ix && output == "permission denied"));
    assert!(matches!(&evs[1], AgentEvent::ToolCallEnd { ix: i, ok: false } if *i == ix));
}

#[test]
fn result_success_emits_usage_and_done() {
    let mut d = ClaudeDecoder::new();
    let dec = d.line(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","usage":{"input_tokens":100,"cache_read_input_tokens":50,"output_tokens":20}}"#,
    );
    assert!(dec.turn_over);
    assert!(dec.events.iter().any(|e| matches!(e, AgentEvent::Usage { input: 150, output: 20 })));
    assert!(matches!(dec.events.last(), Some(AgentEvent::Done)));
}

#[test]
fn result_error_subtype_emits_error_then_done() {
    let mut d = ClaudeDecoder::new();
    let dec = d.line(r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"hit the turn limit"}"#);
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::Error(m) if m.contains("error_max_turns") && m.contains("hit the turn limit")));
    assert!(matches!(dec.events.last(), Some(AgentEvent::Done)));
}

#[test]
fn result_closes_tool_cards_left_open() {
    let mut d = ClaudeDecoder::new();
    events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_9","name":"Bash","input":{"command":"sleep 99"}}]}}"#,
    );
    // Turn ends without a tool_result — the card must not spin forever.
    let dec = d.line(r#"{"type":"result","subtype":"success","is_error":false,"result":"stopped"}"#);
    assert!(dec.events.iter().any(|e| matches!(e, AgentEvent::ToolCallEnd { ok: true, .. })));
    assert!(matches!(dec.events.last(), Some(AgentEvent::Done)));
}

#[test]
fn todo_write_becomes_plan_card() {
    let mut d = ClaudeDecoder::new();
    // Streamed tool_use opens no card — TodoWrite renders as the checklist.
    let evs = events(
        &mut d,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_t","name":"TodoWrite","input":{}}}}"#,
    );
    assert!(evs.is_empty(), "TodoWrite must not open a tool card");

    // The assistant snapshot carries the todos — one Plan event.
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_t","name":"TodoWrite","input":{"todos":[{"content":"scan repo","status":"completed","activeForm":"Scanning"},{"content":"edit files","status":"in_progress","activeForm":"Editing files"},{"content":"run tests","status":"pending","activeForm":"Running tests"}]}}]}}"#,
    );
    let Some(AgentEvent::Plan { steps, .. }) = evs.iter().find(|e| matches!(e, AgentEvent::Plan { .. })) else { panic!("Plan event") };
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].status, crate::model::PlanStatus::Done);
    assert_eq!(steps[0].label.as_str(), "scan repo");
    // In-progress steps show the present-tense activeForm label.
    assert_eq!(steps[1].status, crate::model::PlanStatus::InProgress);
    assert_eq!(steps[1].label.as_str(), "Editing files");
    assert_eq!(steps[2].status, crate::model::PlanStatus::Pending);

    // The tool_result for TodoWrite emits nothing — no card to close.
    let evs = events(
        &mut d,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_t","content":"todos updated"}]}}"#,
    );
    assert!(evs.is_empty(), "TodoWrite result must not emit card events");

    // A second TodoWrite updates the same plan card.
    let evs = events(
        &mut d,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_t2","name":"TodoWrite","input":{"todos":[{"content":"scan repo","status":"completed"},{"content":"edit files","status":"completed"},{"content":"run tests","status":"in_progress"}]}}]}}"#,
    );
    let Some(AgentEvent::Plan { steps, .. }) = evs.iter().find(|e| matches!(e, AgentEvent::Plan { .. })) else { panic!("Plan event") };
    assert_eq!(steps[2].status, crate::model::PlanStatus::InProgress);
}

#[test]
fn backend_for_builds_claude() {
    let p = crate::providers::ProviderInstance::new(crate::providers::ProviderKind::ClaudeCli, "Claude".into());
    assert_eq!(crate::backend::backend_for(&p).name(), "claude-cli");
}

#[test]
fn claude_models_have_no_default_entry() {
    let models = crate::backend::ClaudeCliBackend::new(Vec::new()).models();
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| m.id != "default"));
    assert!(models.iter().any(|m| m.id == "sonnet"));
}

// ── Permission mapping + spawn command (moved out of claude.rs for SLOC) ──

mod command_tests {
    use crate::backend::AccessMode;
    use crate::backend::claude::{ClaudeTurn, build_command, permission_args};

    fn turn(mode: &str, access: AccessMode) -> ClaudeTurn {
        ClaudeTurn {
            prompt: "hi".into(),
            model: "sonnet".into(),
            mode: mode.into(),
            access,
            cwd: std::path::PathBuf::from("/tmp/thread-wt"),
            env: Vec::new(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn agent_maps_access_to_permission_flags() {
        let cases = [
            (AccessMode::Supervised, vec!["--permission-mode", "default"]),
            (AccessMode::AutoAcceptEdits, vec!["--permission-mode", "acceptEdits"]),
            (AccessMode::Auto, vec!["--permission-mode", "acceptEdits"]),
            (AccessMode::FullAccess, vec!["--dangerously-skip-permissions"]),
        ];
        for (access, want) in cases {
            assert_eq!(permission_args(&turn("Agent", access)), want);
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
        t.env = vec![("ANTHROPIC_BASE_URL".to_string(), "https://proxy".to_string())];
        let envs: Vec<_> = build_command(&t).get_envs().map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string()))).collect();
        assert!(envs.contains(&(std::ffi::OsString::from("ANTHROPIC_BASE_URL"), Some(std::ffi::OsString::from("https://proxy")))));
    }

    #[test]
    fn plan_and_ask_stay_read_only() {
        for mode in ["Plan", "Ask"] {
            for access in AccessMode::ALL {
                assert_eq!(permission_args(&turn(mode, access)), vec!["--permission-mode", "plan"]);
            }
        }
    }
}

// ── Auth: `claude auth status` parsing and the login pump ──

mod auth_tests {
    use crate::auth::AuthEvent;
    use crate::backend::claude::{parse_auth_status, pump_login};

    #[test]
    fn auth_status_parses_json() {
        use crate::auth::AuthState;
        assert_eq!(
            parse_auth_status(r#"{"loggedIn":true,"authMethod":"oauth_token","email":"u@x.com"}"#),
            AuthState::SignedIn("u@x.com".into())
        );
        assert_eq!(parse_auth_status(r#"{"loggedIn":true,"authMethod":"oauth_token"}"#), AuthState::SignedIn("oauth_token".into()));
        assert_eq!(parse_auth_status(r#"{"loggedIn":false}"#), AuthState::SignedOut);
        assert_eq!(parse_auth_status("not json"), AuthState::Unknown);
        assert_eq!(parse_auth_status(r#"{"other":1}"#), AuthState::Unknown);
    }

    #[test]
    fn login_pump_surfaces_url_and_code_prompt() {
        let (tx, rx) = std::sync::mpsc::channel();
        let out = b"Opening browser to sign in\xe2\x80\xa6\nIf the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true\nPaste code here if prompted > ".to_vec();
        pump_login(std::io::Cursor::new(out), &tx);
        drop(tx);
        let events: Vec<AuthEvent> = rx.into_iter().collect();
        let prompt = events.iter().find_map(|e| match e {
            AuthEvent::NeedsCode(t) => Some(t.clone()),
            _ => None,
        });
        let prompt = prompt.expect("the paste-back prompt is surfaced");
        assert!(prompt.contains("https://claude.com/cai/oauth/authorize"), "{prompt}");
    }

    #[test]
    fn login_pump_stays_quiet_without_a_prompt() {
        let (tx, rx) = std::sync::mpsc::channel();
        pump_login(std::io::Cursor::new(b"some other output\n".to_vec()), &tx);
        drop(tx);
        assert!(rx.into_iter().next().is_none());
    }
}
