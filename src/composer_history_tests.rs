//! Headless tests for composer prompt history: `push_history` hygiene
//! (trim/dedup/cap), the Up/Down recall session over real input events, and
//! the `prompt_history` disk round-trip. Declared from `composer_history.rs`
//! via `#[path]` — `main.rs` is at the SLOC cap.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::TestAppContext;
use gpui_kit::component::input::Position;
use gpui_kit::test::TestWindowExt;

use super::push_history;
use crate::composer_testutil::{composer_value, open_workspace, type_and_send, until, use_sim};
use crate::model::Chat;
use crate::persist::{load_chats, save_chats};

#[test]
fn push_history_skips_blanks_and_consecutive_dupes() {
    let mut history = Vec::new();
    push_history(&mut history, "first");
    push_history(&mut history, "   ");
    push_history(&mut history, "first");
    push_history(&mut history, "second");
    push_history(&mut history, "first"); // non-consecutive dup is kept
    assert_eq!(history, ["first", "second", "first"]);
}

#[test]
fn push_history_caps_at_100_dropping_oldest() {
    let mut history = vec!["seed".to_string()];
    for i in 0..150 {
        push_history(&mut history, &format!("prompt {i}"));
    }
    assert_eq!(history.first().unwrap(), "prompt 50");
    assert_eq!(history.last().unwrap(), "prompt 149");
}

#[gpui_kit::test]
fn up_down_recalls_sent_prompts(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);
    type_and_send(cx, "second prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);

    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "second prompt");
    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "first prompt");
    cx.update(|window, cx| window.press("down", cx));
    assert_eq!(composer_value(&workspace, cx), "second prompt");
    // Down past the newest leaves the range — the pre-recall draft (empty
    // here) returns and the session ends.
    cx.update(|window, cx| window.press("down", cx));
    assert_eq!(composer_value(&workspace, cx), "");
    workspace.read_with(cx, |ws, _| assert!(ws.history_ix.is_none()));
}

#[gpui_kit::test]
fn recall_restores_an_unsent_draft(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "sent prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);

    cx.update(|window, cx| window.input("draft in progress", cx));
    // Up mid-draft moves the cursor to the start — no recall yet.
    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "draft in progress");
    workspace.read_with(cx, |ws, _| assert!(ws.history_ix.is_none()));
    // At document start, Up recalls; Down past the newest restores the draft.
    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "sent prompt");
    cx.update(|window, cx| window.press("down", cx));
    assert_eq!(composer_value(&workspace, cx), "draft in progress");
    workspace.read_with(cx, |ws, _| assert!(ws.history_ix.is_none()));
}

#[gpui_kit::test]
fn typing_exits_recall(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);
    type_and_send(cx, "second prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);

    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "second prompt");
    cx.update(|window, cx| window.input("x", cx));
    workspace.read_with(cx, |ws, _| assert!(ws.history_ix.is_none(), "typing ends the session"));
    // The edit landed on the recalled text — the session is over, so Down
    // is back to being a plain cursor move, not a step forward.
    cx.update(|window, cx| window.press("down", cx));
    assert_eq!(composer_value(&workspace, cx), "xsecond prompt");
}

#[gpui_kit::test]
fn up_mid_multiline_moves_cursor_not_history(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "sent prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);

    cx.update(|window, cx| window.input("line one\nline two", cx));
    // Cursor sits mid-document on line 2 — Up must move within the text.
    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "line one\nline two");
    workspace.read_with(cx, |ws, app| {
        assert!(ws.history_ix.is_none());
        assert_eq!(ws.composer.read(app).cursor_position().line, 0, "Up moved the cursor up a line");
    });
}

#[gpui_kit::test]
fn up_at_document_start_recalls(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "sent prompt");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);

    cx.update(|window, cx| {
        window.input("line one\nline two", cx);
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |s, cx| s.set_cursor_position(Position::new(0, 0), window, cx));
        });
    });
    cx.update(|window, cx| window.press("up", cx));
    assert_eq!(composer_value(&workspace, cx), "sent prompt");
}

#[test]
fn prompt_history_roundtrips_through_disk() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}-history", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut chat = Chat::new(0, "history chat");
    chat.prompt_history = vec!["first".into(), "second".into()];
    save_chats(&dir, &[chat]);

    let mut next_id = 0;
    let loaded = load_chats(&dir, &mut next_id, true);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].prompt_history, ["first", "second"]);
}

#[test]
fn legacy_chat_file_loads_with_empty_history() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}-legacy-history", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A v1 file written before prompt_history existed — no field at all.
    std::fs::write(dir.join("0.json"), r#"{"v":1,"title":"old","messages":[]}"#).unwrap();

    let mut next_id = 0;
    let loaded = load_chats(&dir, &mut next_id, true);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].prompt_history.is_empty());
}
