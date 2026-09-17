//! Unit tests for chat-title helpers: sanitizing, placeholder detection,
//! first-message derivation, and persistence of the `title_generated` /
//! `title_custom` flags. Headless send-path coverage lives in
//! `chat_title_ui_tests.rs`.

use crate::model::{Chat, ChatMessage, MessageKind, Role};

fn text_msg(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
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

/// `derive_title` is the no-backend fallback: markdown and code fences
/// stripped, whitespace collapsed, first clause kept, ~48-char cap with
/// an ellipsis.
#[test]
fn title_derivation() {
    use super::derive_title;
    // Markdown decoration comes off; the first sentence wins.
    assert_eq!(derive_title("**Fix** the `login` bug. More detail here."), Some("Fix the login bug".to_string()));
    assert_eq!(derive_title("```rust\nlet total = items.len\n```"), Some("let total = items.len".to_string()));
    assert_eq!(derive_title("# Refactor plan\n- step one\n- step two"), Some("Refactor plan".to_string()));
    // Links keep their label; whitespace collapses across lines.
    assert_eq!(derive_title("see [the docs](https://x.dev)   for\ncontext"), Some("see the docs for context".to_string()));
    // Clause boundaries: comma, colon, dash, paren — but not mid-word.
    assert_eq!(derive_title("fix login, then logout"), Some("fix login".to_string()));
    assert_eq!(derive_title("refactor: the parser"), Some("refactor".to_string()));
    assert_eq!(derive_title("use snake_case names"), Some("use snake_case names".to_string()));
    // Long messages cap at a word boundary with an ellipsis.
    let long = derive_title(&"word ".repeat(30)).unwrap();
    assert!(long.ends_with('…'), "truncated titles carry an ellipsis: {long}");
    assert!(long.chars().count() <= 49, "capped near 48 chars: {long}");
    assert!(!long.trim_end_matches('…').ends_with(' '), "no dangling space before the ellipsis");
    // Nothing usable → no title.
    assert_eq!(derive_title("```\n```"), None);
    assert_eq!(derive_title("   \n  "), None);
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

/// `title_generated` and `title_custom` survive a save/load round-trip;
/// a chat file written before the flags existed loads both `false` so it
/// can still earn a title.
#[test]
fn title_flags_persisted() {
    let dir = std::env::temp_dir().join(format!("rixlcode-title-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut chat = Chat::new(1, "Fix the flaky test");
    chat.title_generated = true;
    chat.title_custom = true;
    crate::persist::save_chats(&dir, &[chat]);
    let mut next = 10;
    let loaded = crate::persist::load_chats(&dir, &mut next, false);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].title_generated, "generated flag round-trips");
    assert!(loaded[0].title_custom, "custom flag round-trips");
    // A legacy file without the fields defaults both to false.
    std::fs::write(dir.join("0.json"), r#"{"v":1,"title":"Old chat","messages":[]}"#).unwrap();
    let loaded = crate::persist::load_chats(&dir, &mut next, false);
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].title_generated, "missing field defaults false");
    assert!(!loaded[0].title_custom, "missing field defaults false");
    let _ = std::fs::remove_dir_all(&dir);
}
