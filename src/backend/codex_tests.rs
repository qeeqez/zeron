//! Tests for the codex turn's sandbox/approval mapping and spawn command —
//! sibling file so `codex.rs` stays under the SLOC cap. No real subprocess
//! is ever spawned; `build_command` is inspected directly.

#[cfg(test)]
mod tests {
    use crate::backend::AccessMode;

    use crate::backend::codex::{CodexTurn, approval_of, build_command, sandbox_of};

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
}
