//! Tests for `chat_search` helpers — kept in a sibling file so `chat_search.rs`
//! stays under the 250-SLOC cap.

#[cfg(test)]
mod tests {
    use crate::chat_search::msg_matches;
    use crate::model::{ChatMessage, DiffCard, MessageKind, Role, ToolCall, ToolStatus};

    fn msg(kind: MessageKind) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            kind,
            rating: None,
            usage: None,
            at: std::time::SystemTime::now(),
        }
    }

    #[test]
    fn text_matches_case_insensitive() {
        let m = msg(MessageKind::Text("Hello World".into()));
        assert!(msg_matches(&m, "hello"));
        assert!(!msg_matches(&m, "bye"));
    }

    #[test]
    fn tool_matches_name_detail_output() {
        let m = msg(MessageKind::Tool(ToolCall {
            tool_ix: 0,
            name: "shell".into(),
            detail: "cargo build".into(),
            output: "error[E0308]".into(),
            status: ToolStatus::Done,
            expanded: false,
        }));
        assert!(msg_matches(&m, "shell"));
        assert!(msg_matches(&m, "cargo"));
        assert!(msg_matches(&m, "e0308"));
        assert!(!msg_matches(&m, "missing"));
    }

    #[test]
    fn diff_matches_path_and_hunks() {
        let m = msg(MessageKind::Diff(DiffCard {
            path: "src/main.rs".into(),
            added: 1,
            removed: 0,
            hunks: "+fn main() {}".into(),
            expanded: false,
        }));
        assert!(msg_matches(&m, "main.rs"));
        assert!(msg_matches(&m, "fn main"));
        assert!(!msg_matches(&m, "other"));
    }
}
