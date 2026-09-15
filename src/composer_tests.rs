//! Headless UI tests for the composer menus: `@` file mentions and `/`
//! commands. Drives the real `Workspace` in a test window with native input
//! events. Queue tests live in `composer_queue_tests.rs`; shared helpers in
//! `composer_testutil.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use crate::composer_testutil::{composer_value, open_workspace, type_and_send, until, use_sim};
use crate::model::MessageKind;

#[gpui_kit::test]
fn mention_menu_lists_files_and_inserts_token(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.input("@src/mai", cx);
        assert!(window.try_find("mention-src/main.rs").is_some());
        // Files not matching the query stay out of the menu.
        assert!(window.try_find("mention-Cargo.toml").is_none());
        window.click("mention-src/main.rs", cx);
    });
    assert_eq!(composer_value(&workspace, cx), "@src/main.rs ");
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mention-src/main.rs").is_none());
    });
}

#[gpui_kit::test]
fn mention_menu_respects_word_boundary(cx: &mut TestAppContext) {
    let (_workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.input("user@host", cx);
        assert!(window.try_find("mention-Cargo.toml").is_none());
        window.input(" @", cx);
        assert!(window.try_find("mention-Cargo.toml").is_some());
    });
}

#[gpui_kit::test]
fn slash_menu_filters_and_dispatches(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.input("/", cx);
        assert!(window.try_find("slash-help").is_some());
        assert!(window.try_find("slash-model").is_some());
        // Every row carries its description from SLASH_COMMANDS.
        assert!(window.try_find("slash-help-desc").is_some());
        window.input("he", cx);
        assert!(window.try_find("slash-help").is_some());
        assert!(window.try_find("slash-model").is_none());
        window.click("slash-help", cx);
    });
    assert_eq!(composer_value(&workspace, cx), "");
    let note = workspace.read_with(cx, |ws, _| {
        ws.chats[ws.active]
            .messages
            .iter()
            .rev()
            .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.contains("/help")))
    });
    assert!(note, "expected a /help note message in the chat");
}

#[gpui_kit::test]
fn slash_menu_filters_by_prefix(cx: &mut TestAppContext) {
    let (_workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.input("/st", cx);
        assert!(window.try_find("slash-status").is_some());
        // Prefix match: "status" contains "ta" but doesn't start with it.
        assert!(window.try_find("slash-help").is_none());
        assert!(window.try_find("slash-clear").is_none());
    });
}

#[gpui_kit::test]
fn clear_command_empties_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "hello");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);
    assert!(!workspace.read_with(cx, |ws, _| ws.chats[ws.active].messages.is_empty()));
    type_and_send(cx, "/clear");
    workspace.read_with(cx, |ws, _| {
        assert!(ws.chats[ws.active].messages.is_empty(), "/clear must empty the transcript");
    });
}
