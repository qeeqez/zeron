use crate::backend::AgentEvent;
use crate::backend_parse::parse_codex_line;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_json_yields_nothing() {
        let evs = parse_codex_line("not json");
        assert!(evs.is_empty());
    }

    #[test]
    fn command_execution_start_and_end() {
        let a = parse_codex_line(r#"{"type":"item.started","item":{"id":"x","type":"command_execution","command":"ls"}}"#);
        assert_eq!(a.len(), 1);
        assert!(matches!(&a[0], AgentEvent::ToolCallStart { name, .. } if name == "shell"));

        let b = parse_codex_line(
            r#"{"type":"item.completed","item":{"id":"x","type":"command_execution","aggregated_output":"out","exit_code":0}}"#,
        );
        assert_eq!(b.len(), 2);
        assert!(matches!(&b[0], AgentEvent::ToolCallDelta { output, .. } if output == "out"));
        assert!(matches!(&b[1], AgentEvent::ToolCallEnd { ok: true, .. }));
    }

    #[test]
    fn agent_message_starts_new_bubble() {
        let evs = parse_codex_line(r#"{"type":"item.started","item":{"id":"m","type":"agent_message"}}"#);
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], AgentEvent::TextStart));
    }

    #[test]
    fn error_variants() {
        let evs = parse_codex_line(r#"{"type":"error","message":"boom"}"#);
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], AgentEvent::Error(e) if e == "boom"));

        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"e","type":"error","message":"item err"}}"#);
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], AgentEvent::Error(e) if e == "item err"));
    }

    #[test]
    fn turn_completed_emits_usage_and_done() {
        let evs = parse_codex_line(r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":20}}"#);
        assert_eq!(evs.len(), 2);
        assert!(matches!(&evs[0], AgentEvent::Usage { input: 10, output: 20 }));
        assert!(matches!(&evs[1], AgentEvent::Done));
    }

    #[test]
    fn empty_output_skips_delta() {
        let evs = parse_codex_line(
            r#"{"type":"item.completed","item":{"id":"x","type":"command_execution","aggregated_output":"","exit_code":0}}"#,
        );
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], AgentEvent::ToolCallEnd { ok: true, .. }));
    }

    #[test]
    fn same_item_id_routes_to_same_ix() {
        let a = parse_codex_line(r#"{"type":"item.started","item":{"id":"x","type":"command_execution","command":"ls"}}"#);
        let b = parse_codex_line(
            r#"{"type":"item.completed","item":{"id":"x","type":"command_execution","aggregated_output":"","exit_code":0}}"#,
        );
        let ix_a = match &a[0] {
            AgentEvent::ToolCallStart { ix, .. } => *ix,
            _ => panic!(),
        };
        let ix_b = match &b[0] {
            AgentEvent::ToolCallEnd { ix, .. } => *ix,
            _ => panic!(),
        };
        assert_eq!(ix_a, ix_b);
    }
}

#[cfg(test)]
mod reasoning_tests {
    use super::*;

    #[test]
    fn reasoning_string_text() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":"thinking…"}}"#);
        assert_eq!(evs.len(), 3);
        assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, .. } if name == "thinking"));
        assert!(matches!(&evs[1], AgentEvent::ToolCallDelta { output, .. } if output == "thinking…"));
        assert!(matches!(&evs[2], AgentEvent::ToolCallEnd { ok: true, .. }));
    }

    #[test]
    fn reasoning_array_text() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":[{"text":"a"},{"text":"b"}]}}"#);
        assert_eq!(evs.len(), 3);
        assert!(matches!(&evs[1], AgentEvent::ToolCallDelta { output, .. } if output == "a\nb"));
    }

    #[test]
    fn reasoning_empty_yields_nothing() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":""}}"#);
        assert!(evs.is_empty());
    }
}

#[cfg(test)]
mod file_change_tests {
    use super::*;

    #[test]
    fn non_completed_yields_nothing() {
        let evs =
            parse_codex_line(r#"{"type":"item.completed","item":{"id":"f","type":"file_change","status":"in_progress","changes":[]}}"#);
        assert!(evs.is_empty());
    }

    #[test]
    fn missing_changes_yields_nothing() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"f","type":"file_change","status":"completed"}}"#);
        assert!(evs.is_empty());
    }
}

#[cfg(test)]
mod turn_tests {
    use super::*;

    #[test]
    fn turn_completed_missing_usage() {
        let evs = parse_codex_line(r#"{"type":"turn.completed"}"#);
        assert_eq!(evs.len(), 2);
        assert!(matches!(&evs[0], AgentEvent::Usage { input: 0, output: 0 }));
        assert!(matches!(&evs[1], AgentEvent::Done));
    }

    #[test]
    fn turn_failed_missing_message() {
        let evs = parse_codex_line(r#"{"type":"turn.failed"}"#);
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], AgentEvent::Error(e) if e == "turn failed"));
    }
}

#[cfg(test)]
mod agent_message_tests {
    use super::*;

    #[test]
    fn agent_message_completed_without_start() {
        // A completed agent_message with no prior item.started still appends
        // to the last text bubble — apply_text_delta creates it if needed.
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"m","type":"agent_message","text":"hi"}}"#);
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "hi"));
    }
}
