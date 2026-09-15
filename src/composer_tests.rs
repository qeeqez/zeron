//! Headless UI tests for the composer: `@` file mentions and `/` commands.
//! Drives the real `Workspace` in a test window with native input events.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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

/// Send `text` through the real input path: type, then Enter.
fn type_and_send(cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        window.input(text, cx);
        window.press("enter", cx);
    });
}

/// Advance the test clock until `cond` holds or the budget runs out.
fn until(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| cond(ws)) {
            return;
        }
    }
    panic!("condition never held");
}

/// Count user messages whose text contains `needle`.
fn user_msgs(ws: &Workspace, needle: &str) -> usize {
    ws.chats[ws.active]
        .messages
        .iter()
        .filter(|m| m.role == crate::model::Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains(needle)))
        .count()
}

/// Point the workspace at the sim backend so sends complete on the test clock.
fn use_sim(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(crate::backend::SimBackend);
        });
    });
}

#[gpui_kit::test]
fn queued_message_sends_after_reply_finishes(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first");
    assert!(workspace.read_with(cx, |ws, _| ws.chats[ws.active].running));

    // Typing + Enter mid-reply queues instead of dropping the message.
    type_and_send(cx, "second");
    assert_eq!(composer_value(&workspace, cx), "");
    assert_eq!(workspace.read_with(cx, |ws, _| user_msgs(ws, "second")), 0, "queued text must not post mid-reply");
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("queued-0").is_some(), "queued row should render");
    });

    // Turn 1 ends → the queued message sends itself and turn 2 runs.
    until(&workspace, cx, |ws| user_msgs(ws, "second") == 1 && ws.chats[ws.active].running);
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);
    assert_eq!(workspace.read_with(cx, |ws, _| user_msgs(ws, "second")), 1);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("queued-0").is_none(), "queue should be empty");
    });
}

#[gpui_kit::test]
fn dequeue_drops_queued_message(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first");
    type_and_send(cx, "second");
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("dequeue-0", cx);
    });
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);
    assert_eq!(workspace.read_with(cx, |ws, _| user_msgs(ws, "second")), 0, "dequeued message must never send");
}

/// Clicking a queued row reopens it in the composer; Enter commits the
/// edit back into the queue at its position, and the drain sends the
/// edited text — never the original.
#[gpui_kit::test]
fn queued_message_edits_via_composer(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first");
    type_and_send(cx, "second");
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("queued-edit-0", cx);
    });
    assert_eq!(composer_value(&workspace, cx), "second", "queued text loads into the composer");
    assert!(workspace.read_with(cx, |ws, _| ws.send_queue.queued(ws.chats[ws.active].id).is_empty()));

    cx.update(|window, cx| {
        window.press("cmd-a", cx);
        window.input("second edited", cx);
        window.press("enter", cx);
    });
    let queued = workspace.read_with(cx, |ws, _| ws.send_queue.queued(ws.chats[ws.active].id));
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].text, "second edited");
    assert_eq!(composer_value(&workspace, cx), "", "composer restores the pre-edit draft");

    until(&workspace, cx, |ws| user_msgs(ws, "second edited") == 1);
    assert_eq!(
        workspace.read_with(cx, |ws, _| {
            ws.chats[ws.active]
                .messages
                .iter()
                .filter(|m| matches!(&m.kind, MessageKind::Text(t) if t.as_str() == "second"))
                .count()
        }),
        0,
        "the pre-edit text must never send"
    );
}

/// The queue survives a restart: a second window over the same project
/// adopts the persisted queue on first render.
#[gpui_kit::test]
fn queued_messages_persist_across_windows(cx: &mut TestAppContext) {
    let (ws_a, cx) = open_workspace(cx);
    use_sim(&ws_a, cx);
    type_and_send(cx, "first");
    type_and_send(cx, "queued-before-quit");
    assert_eq!(ws_a.read_with(cx, |ws, _| ws.send_queue.queued(ws.chats[ws.active].id).len()), 1);

    // A new window on the same project = a restart: same chats dir, fresh
    // SendQueue. First render hydrates it from disk.
    let mut ws_b = None;
    let window_b = cx.open_window(size(px(1024.), px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws_b = Some(view.clone());
        Root::new(view, window, cx)
    });
    let ws_b = ws_b.unwrap();
    let mut cx_b = VisualTestContext::from_window(window_b.into(), cx);
    cx_b.update(|window, cx| window.render_frame(cx));
    let queued = ws_b.read_with(&cx_b, |ws, _| ws.send_queue.queued(ws.chats[ws.active].id));
    assert_eq!(queued.len(), 1, "restarted window must adopt the persisted queue");
    assert_eq!(queued[0].text, "queued-before-quit");
}

/// The queued row's controls work headlessly: the arrows reorder, the send
/// icon jumps an item to the front, and the drain honors the new order.
#[gpui_kit::test]
fn queued_rows_reorder_and_send_now(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first");
    type_and_send(cx, "aaa");
    type_and_send(cx, "bbb");
    type_and_send(cx, "ccc");
    let order = |ws: &Workspace| ws.send_queue.queued(ws.chats[ws.active].id).iter().map(|i| i.text.clone()).collect::<Vec<_>>();

    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("queue-up-2", cx); // ccc above bbb
    });
    assert_eq!(workspace.read_with(cx, |ws, _| order(ws)), ["aaa", "ccc", "bbb"]);

    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("queue-send-1", cx); // bbb sends next
    });
    assert_eq!(workspace.read_with(cx, |ws, _| order(ws)), ["bbb", "aaa", "ccc"]);

    // The drain sends in the new order: bbb's user message lands first.
    until(&workspace, cx, |ws| user_msgs(ws, "ccc") == 1 && !ws.chats[ws.active].running);
    let pos = |ws: &Workspace, needle: &str| {
        ws.chats[ws.active]
            .messages
            .iter()
            .position(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains(needle)))
            .unwrap()
    };
    let (b, a, c) = workspace.read_with(cx, |ws, _| (pos(ws, "bbb"), pos(ws, "aaa"), pos(ws, "ccc")));
    assert!(b < a && a < c, "queue order must drive send order");
}
