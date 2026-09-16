//! Unit tests for chat-title helpers: sanitizing, placeholder detection,
//! and persistence of the `title_generated` flag. Headless send-path
//! coverage lives in `chat_title_ui_tests.rs`.

use crate::model::{Chat, ChatMessage, MessageKind, Role};

fn text_msg(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

/// `clean_title` strips markdown decoration and trailing punctuation and
/// caps the length; `provisional_title` is the first line, 40 chars.
#[test]
fn title_cleanup() {
    use super::{clean_title, provisional_title};
    assert_eq!(clean_title("\"Fix the flaky test.\"\nnotes"), Some("Fix the flaky test".to_string()));
    assert_eq!(clean_title("- `Parser refactor:`"), Some("Parser refactor".to_string()));
    assert_eq!(clean_title("   \n  "), None);
    let long = "word ".repeat(30);
    assert!(clean_title(&long).unwrap().chars().count() <= 60, "capped at 60 chars");
    assert_eq!(provisional_title("first line\nsecond"), "first line");
    assert_eq!(provisional_title(&"x".repeat(50)).chars().count(), 40);
}

/// Only the placeholder titles qualify for generation — "New chat" or the
/// truncated first prompt; a user's own title never does.
#[test]
fn placeholder_detection() {
    use super::has_placeholder_title;
    let mut chat = Chat::new(1, "New chat");
    assert!(has_placeholder_title(&chat), "untouched chat qualifies");
    chat.messages = std::rc::Rc::new(vec![text_msg(Role::User, "fix the login bug")]);
    chat.title = "fix the login bug".into();
    assert!(has_placeholder_title(&chat), "truncated-prompt placeholder qualifies");
    chat.title = "My custom name".into();
    assert!(!has_placeholder_title(&chat), "a manual title is left alone");
    chat.title = "".into();
    assert!(!has_placeholder_title(&chat), "an empty title is left alone");
}

/// `title_generated` survives a save/load round-trip; a chat file written
/// before the flag existed loads `false` so it can still earn a title.
#[test]
fn generated_flag_persisted() {
    let dir = std::env::temp_dir().join(format!("rixlcode-title-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut chat = Chat::new(1, "Fix the flaky test");
    chat.title_generated = true;
    crate::persist::save_chats(&dir, &[chat]);
    let mut next = 10;
    let loaded = crate::persist::load_chats(&dir, &mut next, false);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].title_generated, "flag round-trips");
    // A legacy file without the field defaults to false.
    std::fs::write(dir.join("0.json"), r#"{"v":1,"title":"Old chat","messages":[]}"#).unwrap();
    let loaded = crate::persist::load_chats(&dir, &mut next, false);
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].title_generated, "missing field defaults false");
    let _ = std::fs::remove_dir_all(&dir);
}
