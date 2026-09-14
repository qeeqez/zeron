//! Disk round-trip tests for `persist::save_chats`/`load_chats` — kept in a
//! sibling file so `persist.rs` stays under the 250-SLOC cap. Serde-default
//! tests for `StoredChat`/`Settings` live in `persist.rs` itself.

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::time::SystemTime;

    use crate::model::{Chat, ChatMessage, MessageKind, Role, ToolCall, ToolStatus, Usage};
    use crate::persist::{load_chats, save_chats};

    /// A fresh throwaway chats dir per test — save/load take the dir
    /// explicitly, so tests never touch the real `~/.rixl/rixlcode`.
    fn temp_chats_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn msg(role: Role, kind: MessageKind) -> ChatMessage {
        ChatMessage {
            role,
            kind,
            rating: None,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        }
    }

    #[test]
    fn messages_roundtrip_through_disk() {
        let dir = temp_chats_dir("roundtrip");
        let mut chat = Chat::new(0, "saved chat");
        chat.messages = Rc::new(vec![
            ChatMessage {
                attachments: vec!["src/main.rs".into()],
                ..msg(Role::User, MessageKind::Text("fix the build".into()))
            },
            msg(
                Role::Assistant,
                MessageKind::Tool(ToolCall {
                    tool_ix: 0,
                    name: "shell".into(),
                    detail: "cargo build".into(),
                    output: "ok".into(),
                    status: ToolStatus::Done,
                    expanded: false,
                }),
            ),
            ChatMessage {
                rating: Some(true),
                usage: Some(Usage { input: 12, output: 34 }),
                ..msg(Role::Assistant, MessageKind::Text("done".into()))
            },
        ]);
        save_chats(&dir, &[chat]);

        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id);
        assert_eq!(loaded.len(), 1);
        let chat = &loaded[0];
        assert_eq!(chat.title, "saved chat");
        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[0].role, Role::User);
        assert!(matches!(&chat.messages[0].kind, MessageKind::Text(t) if t.as_str() == "fix the build"));
        assert_eq!(chat.messages[0].attachments.len(), 1);
        assert!(
            matches!(&chat.messages[1].kind, MessageKind::Tool(t) if t.status == ToolStatus::Done && t.detail.as_str() == "cargo build")
        );
        assert_eq!(chat.messages[2].rating, Some(true));
        assert_eq!(chat.messages[2].usage.map(|u| (u.input, u.output)), Some((12, 34)));
        // Loaded chats get fresh ids — new chats must not collide with them.
        assert_eq!(next_id, 1);
    }

    #[test]
    fn running_state_does_not_survive_reload() {
        let dir = temp_chats_dir("running");
        // A chat saved mid-turn reopens idle: the turn's task and child died
        // with the process, so `running` and any in-flight tool call must not
        // resurrect.
        let mut chat = Chat::new(0, "interrupted");
        chat.running = true;
        chat.messages = Rc::new(vec![
            msg(Role::User, MessageKind::Text("go".into())),
            msg(
                Role::Assistant,
                MessageKind::Tool(ToolCall {
                    tool_ix: 0,
                    name: "shell".into(),
                    detail: "make".into(),
                    output: "".into(),
                    status: ToolStatus::Running,
                    expanded: false,
                }),
            ),
        ]);
        save_chats(&dir, &[chat]);

        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id);
        assert_eq!(loaded.len(), 1);
        assert!(!loaded[0].running, "a dead turn must not restore as running");
        assert!(
            matches!(&loaded[0].messages[1].kind, MessageKind::Tool(t) if t.status == ToolStatus::Failed),
            "interrupted tool call must restore as failed, not spinning"
        );
    }

    #[test]
    fn deleted_chat_files_do_not_resurrect() {
        let dir = temp_chats_dir("deleted");
        save_chats(&dir, &[Chat::new(0, "a"), Chat::new(1, "b")]);
        save_chats(&dir, &[Chat::new(0, "a")]);
        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "a");
    }

    #[test]
    fn unknown_version_file_is_kept_as_bak() {
        let dir = temp_chats_dir("bak");
        let path = dir.join("0.json");
        std::fs::write(&path, r#"{"v":99,"title":"future","messages":[]}"#).unwrap();
        let mut next_id = 0;
        assert!(load_chats(&dir, &mut next_id).is_empty());
        assert!(!path.exists());
        assert!(path.with_extension("json.bak").exists(), "unreadable file must be preserved");
    }
}
