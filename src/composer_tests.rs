//! Headless UI tests for the composer menus: `@` file mentions and `/`
//! commands. Drives the real `Workspace` in a test window with native input
//! events. Queue tests live in `composer_queue_tests.rs`; shared helpers in
//! `composer_testutil.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::input::Paste;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ClipboardItem, Image, ImageFormat, TestAppContext};

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

#[gpui_kit::test]
fn image_attachment_shows_thumbnail_chip(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_attachments(vec![std::path::PathBuf::from("/tmp/shot.png"), std::path::PathBuf::from("/tmp/notes.txt")], cx);
        });
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        // The .png previews as a thumbnail; the .txt keeps the file icon.
        assert!(window.try_find("attach-thumb-0").is_some());
        assert!(window.try_find("attach-thumb-1").is_none());
    });
}

#[gpui_kit::test]
fn paste_image_attaches_a_saved_file(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, vec![1, 2, 3])));
    cx.dispatch_action(Paste);
    let attachments = workspace.read_with(cx, |ws, _| ws.chats[ws.active].attachments.clone());
    assert_eq!(attachments.len(), 1, "pasted image should attach");
    let path = attachments[0].to_string();
    assert!(path.ends_with(".png"), "saved paste keeps its format: {path}");
    assert!(std::path::Path::new(&path).exists(), "paste wrote the file: {path}");
    // The paste was consumed — nothing lands in the composer text.
    assert_eq!(composer_value(&workspace, cx), "");
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("attach-thumb-0").is_some());
    });
}

#[gpui_kit::test]
fn paste_text_still_reaches_the_composer(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("plain text".into()));
    cx.dispatch_action(Paste);
    assert_eq!(composer_value(&workspace, cx), "plain text");
    assert!(workspace.read_with(cx, |ws, _| ws.chats[ws.active].attachments.is_empty()));
}
