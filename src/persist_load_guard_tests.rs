//! Multi-window guard rails for lazy loading: a foreign file must never
//! graft onto a pending chat's metadata save, messages landed while the
//! file was unreadable must still persist, and a diverged pending chat's
//! live transcript must survive a late-arriving hydrate. Declared from
//! `persist_load.rs` — `persist_load_tests.rs` is at the SLOC cap.

use std::rc::Rc;

use super::persist_load_tests::{msg, seeded_chat, temp_dir};
use crate::model::{MessageKind, ToolCall, ToolStatus};
use crate::persist::{load_chats, save_chats};

#[test]
fn foreign_slot_file_is_not_grafted() {
    let dir = temp_dir("foreign");
    let ours_at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_001);
    let foreign_at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_002);
    let mut tool = msg("call");
    tool.kind = MessageKind::Tool(ToolCall {
        tool_ix: 0,
        name: "shell".into(),
        detail: "make".into(),
        output: "".into(),
        status: ToolStatus::Running,
        expanded: false,
    });
    let mut ours = seeded_chat(0, "ours", vec![msg("one"), tool]);
    ours.created_at = ours_at;
    let mut foreign = seeded_chat(1, "foreign", vec![msg("f1"), msg("f2"), msg("f3")]);
    foreign.created_at = foreign_at;
    save_chats(&dir, &[ours, foreign]);
    let mut next_id = 0;
    let loaded = load_chats(&dir, &mut next_id, false);
    // Another window's slot map differs: the foreign transcript now sits
    // at our chat's slot while ours drifted to slot 1.
    std::fs::rename(dir.join("0.json"), dir.join("tmp.json")).unwrap();
    std::fs::rename(dir.join("1.json"), dir.join("0.json")).unwrap();
    std::fs::rename(dir.join("tmp.json"), dir.join("1.json")).unwrap();
    // Save only the pending "ours" chat — its Running tool triggers the
    // foreign-turn re-read of slot 0, which must not graft the foreign
    // transcript onto our metadata.
    save_chats(&dir, &loaded[..1]);
    let stored = crate::persist::read_stored(&dir.join("0.json")).unwrap();
    assert_eq!(stored.title, "ours");
    assert_eq!(stored.created_at, ours_at);
    assert_eq!(stored.messages.len(), 2, "the foreign transcript must not be grafted");
    assert!(matches!(&stored.messages[0].kind, MessageKind::Text(t) if t.as_str() == "one"));
}

#[test]
fn messages_added_while_the_file_is_gone_still_persist() {
    let dir = temp_dir("vanished");
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![msg("one")])]);
    let mut next_id = 0;
    let mut loaded = load_chats(&dir, &mut next_id, false);
    std::fs::remove_file(dir.join("0.json")).unwrap();
    // Hydration keeps failing while the file is gone — a send landing in
    // the meantime must still persist, not ride the `continue` skip that
    // protects an untouched pending chat's file.
    assert!(!crate::persist::hydrate_chat(&mut loaded[0], &dir));
    Rc::make_mut(&mut loaded[0].messages).push(msg("typed with the file gone"));
    save_chats(&dir, &loaded);
    let stored = crate::persist::read_stored(&dir.join("0.json")).expect("the live message must be written");
    assert_eq!(stored.messages.len(), 1);
    assert!(matches!(&stored.messages[0].kind, MessageKind::Text(t) if t.as_str() == "typed with the file gone"));
}

#[test]
fn diverged_pending_chat_is_not_clobbered_by_hydrate() {
    let dir = temp_dir("diverged");
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![msg("one")])]);
    let mut next_id = 0;
    let mut loaded = load_chats(&dir, &mut next_id, false);
    std::fs::remove_file(dir.join("0.json")).unwrap();
    assert!(!crate::persist::hydrate_chat(&mut loaded[0], &dir));
    Rc::make_mut(&mut loaded[0].messages).push(msg("live"));
    // A same-created_at file reappears (another window's stale view) —
    // hydrating must not drop the live message: once a pending chat
    // carries real messages, memory is authoritative.
    let mut revived = seeded_chat(0, "alpha", vec![msg("revived")]);
    revived.created_at = loaded[0].created_at;
    save_chats(&dir, &[revived]);
    assert!(crate::persist::hydrate_chat(&mut loaded[0], &dir));
    assert!(loaded[0].pending_load.is_none());
    assert_eq!(loaded[0].messages.len(), 1);
    assert!(matches!(&loaded[0].messages[0].kind, MessageKind::Text(t) if t.as_str() == "live"));
}
