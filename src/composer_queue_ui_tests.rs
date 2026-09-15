//! Headless UI tests for the composer's queued-message rows: send-after-reply,
//! dequeue, click-to-edit, persistence across windows, and reorder/send-now.
//! Logic-level queue tests live in `composer_queue_tests.rs`; shared helpers in
//! `composer_testutil.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, VisualTestContext, px, size};

use crate::composer_testutil::{composer_value, open_workspace, type_and_send, until, use_sim, user_msgs};
use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

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
