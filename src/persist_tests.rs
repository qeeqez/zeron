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
            alternatives: vec![],
            role,
            kind,
            rating: None,
            bookmarked: false,
            pinned: false,
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
                bookmarked: false,
                pinned: false,
                usage: Some(Usage { input: 12, output: 34 }),
                ..msg(Role::Assistant, MessageKind::Text("done".into()))
            },
        ]);
        save_chats(&dir, &[chat]);

        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id, true);
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
        let loaded = load_chats(&dir, &mut next_id, true);
        assert_eq!(loaded.len(), 1);
        assert!(!loaded[0].running, "a dead turn must not restore as running");
        assert!(
            matches!(&loaded[0].messages[1].kind, MessageKind::Tool(t) if t.status == ToolStatus::Failed),
            "interrupted tool call must restore as failed, not spinning"
        );
    }

    #[test]
    fn running_tool_survives_warm_reload() {
        let dir = temp_chats_dir("warm");
        // A second window loading while another window's turn is live must
        // not rewrite the live tool's status — a later save would persist
        // the false failure over the real result.
        let mut chat = Chat::new(0, "live turn");
        chat.messages = Rc::new(vec![msg(
            Role::Assistant,
            MessageKind::Tool(ToolCall {
                tool_ix: 0,
                name: "shell".into(),
                detail: "make".into(),
                output: "".into(),
                status: ToolStatus::Running,
                expanded: false,
            }),
        )]);
        save_chats(&dir, &[chat]);

        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id, false);
        assert_eq!(loaded.len(), 1);
        assert!(
            matches!(&loaded[0].messages[0].kind, MessageKind::Tool(t) if t.status == ToolStatus::Running),
            "a live turn's tool must keep its Running status"
        );
    }

    #[test]
    fn deleted_chat_files_do_not_resurrect() {
        let dir = temp_chats_dir("deleted");
        save_chats(&dir, &[Chat::new(0, "a"), Chat::new(1, "b")]);
        save_chats(&dir, &[Chat::new(0, "a")]);
        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id, true);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "a");
    }

    #[test]
    fn unknown_version_file_is_kept_as_bak() {
        let dir = temp_chats_dir("bak");
        let path = dir.join("0.json");
        std::fs::write(&path, r#"{"v":99,"title":"future","messages":[]}"#).unwrap();
        let mut next_id = 0;
        assert!(load_chats(&dir, &mut next_id, true).is_empty());
        assert!(!path.exists());
        assert!(path.with_extension("json.bak").exists(), "unreadable file must be preserved");
    }

    #[test]
    fn stored_chat_defaults_missing_fields() {
        // Early v1 files lack pinned/archived/draft/created_at — they must
        // parse with defaults instead of dropping the chat.
        let json = r#"{"v":1,"title":"t","messages":[]}"#;
        let s: crate::persist::StoredChat = serde_json::from_str(json).unwrap();
        assert!(!s.pinned && !s.archived && s.draft.is_empty());
    }

    #[test]
    fn settings_defaults_missing_fields() {
        // A file with only `model` must not reset the rest.
        let s: crate::persist::Settings = serde_json::from_str(r#"{"model":"gpt-5"}"#).unwrap();
        assert_eq!(s.legacy_model, "gpt-5");
        assert_eq!(s.font_size, 14);
        assert!(s.notify_on_done);
        assert!(s.notify_sound);
        assert!(s.notify_background);
        // Thread defaults are unset until the user picks them.
        assert!(s.default_model.provider_instance_id.is_empty());
        assert!(s.default_permissions.is_empty());
        assert!(s.default_workspace.is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let s = crate::persist::Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: crate::persist::Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.selected_model, s.selected_model);
        assert_eq!(back.font_size, s.font_size);
        assert_eq!(back.notify_sound, s.notify_sound);
        assert_eq!(back.notify_background, s.notify_background);
    }

    #[test]
    fn thread_fields_roundtrip_through_disk() {
        let dir = temp_chats_dir("thread-fields");
        let mut chat = Chat::new(0, "wt chat");
        chat.provider = "claude-cli".into();
        chat.model = "opus".into();
        chat.access = Some(crate::backend::AccessMode::Supervised);
        chat.effort = Some("high".into());
        chat.workdir = "/repo/.worktrees/thread-0".into();
        chat.worktree = true;
        save_chats(&dir, &[chat]);

        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id, true);
        let chat = &loaded[0];
        assert_eq!(chat.provider, "claude-cli");
        assert_eq!(chat.model, "opus");
        assert_eq!(chat.access, Some(crate::backend::AccessMode::Supervised));
        assert_eq!(chat.effort.as_deref(), Some("high"));
        assert_eq!(chat.workdir, "/repo/.worktrees/thread-0");
        assert!(chat.worktree);
    }

    #[test]
    fn draft_roundtrips_through_disk() {
        let dir = temp_chats_dir("draft");
        let mut chat = Chat::new(0, "drafty");
        chat.draft = "half-written reply".into();
        save_chats(&dir, &[chat]);

        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id, true);
        assert_eq!(loaded[0].draft, "half-written reply");
    }

    #[test]
    fn empty_draft_is_not_serialized() {
        let dir = temp_chats_dir("draft-empty");
        save_chats(&dir, &[Chat::new(0, "plain")]);
        let json = std::fs::read_to_string(dir.join("0.json")).unwrap();
        assert!(!json.contains("\"draft\""), "empty drafts keep the key out of the file: {json}");
    }

    #[test]
    fn legacy_chat_file_loads_with_empty_thread_fields() {
        let dir = temp_chats_dir("legacy-fields");
        std::fs::write(dir.join("0.json"), r#"{"v":1,"title":"old","messages":[]}"#).unwrap();
        let mut next_id = 0;
        let loaded = load_chats(&dir, &mut next_id, true);
        let chat = &loaded[0];
        assert!(chat.provider.is_empty() && chat.model.is_empty());
        assert!(chat.access.is_none());
        assert!(chat.effort.is_none());
        assert!(chat.workdir.is_empty() && !chat.worktree);
    }
}
