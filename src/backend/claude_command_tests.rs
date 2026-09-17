//! Tests for the `claude -p` spawn command: permission-mode mapping,
//! `--resume` on bound chats, env injection, and the spawn workdir —
//! sibling file so `claude_tests.rs` stays under the SLOC cap. No real
//! subprocess is ever spawned; `build_command` is inspected directly.

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
            resume: None,
            instructions: None,
            env: Vec::new(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn args(turn: &ClaudeTurn) -> Vec<std::ffi::OsString> {
        build_command(turn).get_args().map(|a| a.to_os_string()).collect()
    }

    #[test]
    fn bound_turn_resumes_its_session() {
        let mut t = turn("Agent", AccessMode::Auto);
        t.resume = Some("sess-1".into());
        let argv = args(&t);
        let pos = argv.iter().position(|a| a == "--resume").expect("argv was: {argv:?}");
        assert_eq!(argv[pos + 1], "sess-1");
    }

    #[test]
    fn fresh_turn_persists_its_session() {
        // No --resume and no --no-session-persistence: the session must
        // land in claude's history so the chat can resume it later.
        let argv = args(&turn("Agent", AccessMode::Auto));
        assert!(!argv.iter().any(|a| a == "--resume"), "argv was: {argv:?}");
        assert!(!argv.iter().any(|a| a == "--no-session-persistence"), "argv was: {argv:?}");
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
