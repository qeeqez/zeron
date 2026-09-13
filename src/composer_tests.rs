//! Headless UI tests for the composer: `@` file mentions and `/` commands.
//! Drives the real `Workspace` in a test window with native input events.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::model::MessageKind;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let window = cx.open_window(size(px(1024.), px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    let workspace = workspace.unwrap();
    // The project-file scan runs on the background executor — poll until it
    // lands instead of assuming run_until_parked covers real threads.
    for _ in 0..200 {
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| !ws.project_files.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(workspace.read_with(cx, |ws, _| !ws.project_files.is_empty()), "project file scan never landed");
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |composer, cx| composer.focus(window, cx));
        });
    });
    (workspace, cx)
}

fn composer_value(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    workspace.read_with(cx, |ws, app| ws.composer.read(app).value().to_string())
}

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
