//! Headless tests for the send queue: per-workspace scoping, slash commands
//! deferred behind a running reply, and per-message attachment snapshots.
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
    for _ in 0..200 {
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| !ws.project_files.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |composer, cx| composer.focus(window, cx));
        });
    });
    (workspace, cx)
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
        .filter(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains(needle)))
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

/// Two windows whose chats share id 0 must not share queues — the queue is
/// workspace state, not a thread-local keyed by chat id alone.
#[gpui_kit::test]
fn queue_is_scoped_per_workspace(cx: &mut TestAppContext) {
    let (ws_a, cx) = open_workspace(cx);
    use_sim(&ws_a, cx);

    // A second window over the same project loads the same chat ids.
    let mut ws_b = None;
    let _window_b = cx.open_window(size(px(1024.), px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws_b = Some(view.clone());
        Root::new(view, window, cx)
    });
    let ws_b = ws_b.unwrap();
    let chat_id = ws_a.read_with(cx, |ws, _| ws.chats[ws.active].id);
    assert_eq!(ws_b.read_with(cx, |ws, _| ws.chats[ws.active].id), chat_id, "test needs both windows on the same chat id");

    // Queue a message behind a running reply in window A.
    type_and_send(cx, "first");
    type_and_send(cx, "queued-in-a");
    assert_eq!(ws_a.read_with(cx, |ws, _| ws.send_queue.queued(chat_id).len()), 1);
    assert!(ws_b.read_with(cx, |ws, _| ws.send_queue.queued(chat_id).is_empty()), "window B must not see A's queue");

    // The drain sends it in A only; B's chat and queue stay untouched.
    until(&ws_a, cx, |ws| user_msgs(ws, "queued-in-a") == 1);
    assert_eq!(ws_b.read_with(cx, |ws, _| user_msgs(ws, "queued-in-a")), 0);
    assert!(ws_b.read_with(cx, |ws, _| ws.send_queue.queued(chat_id).is_empty()));
}

/// `/help` submitted mid-stream must queue like text: its note lands after
/// the reply instead of becoming the last message and absorbing the stream.
#[gpui_kit::test]
fn slash_command_queued_mid_stream_keeps_reply_intact(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first");

    // Wait until the reply is actually streaming, then submit /help.
    until(&workspace, cx, |ws| {
        ws.chats[ws.active].running && matches!(ws.chats[ws.active].messages.last(), Some(m) if matches!(m.kind, MessageKind::Text(_)))
    });
    type_and_send(cx, "/help");

    // Deferred: the note must not exist while the reply is in flight.
    assert!(workspace.read_with(cx, |ws, _| {
        !ws.chats[ws.active]
            .messages
            .iter()
            .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.contains("Commands:")))
    }));
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("queued-0").is_some(), "/help should sit in the queue");
    });

    // Turn ends → the queued command runs and appends its own note.
    until(&workspace, cx, |ws| {
        !ws.chats[ws.active].running
            && matches!(ws.chats[ws.active].messages.last(), Some(m) if matches!(&m.kind, MessageKind::Text(t) if t.contains("Commands:")))
    });
    let msgs = workspace.read_with(cx, |ws, _| {
        ws.chats[ws.active]
            .messages
            .iter()
            .map(|m| match &m.kind {
                MessageKind::Text(t) => t.to_string(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
    });
    // The reply is its own bubble — the note neither absorbed it nor was
    // overwritten by it.
    assert!(msgs.iter().any(|t| t.contains("build is **green**") || t.contains("command **failed**")), "reply text missing");
    assert!(!msgs[..msgs.len() - 1].iter().any(|t| t.contains("Commands:")), "note merged into the reply");
}

/// Enqueueing snapshots the attachment list: the queued message keeps what
/// was attached at submit time, and later chips don't leak into it.
#[gpui_kit::test]
fn queued_message_snapshots_attachments(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    let attach = |cx: &mut VisualTestContext, path: &str| {
        cx.update(|_, cx| {
            workspace.update(cx, |ws, cx| ws.add_attachments(vec![std::path::PathBuf::from(path)], cx));
        });
    };

    attach(cx, "/tmp/a.rs");
    type_and_send(cx, "first");
    assert!(workspace.read_with(cx, |ws, _| ws.chats[ws.active].attachments.is_empty()), "send consumes attachments");

    attach(cx, "/tmp/b.rs");
    type_and_send(cx, "second");
    // The chip list clears on enqueue — the snapshot moved into the queue.
    assert!(workspace.read_with(cx, |ws, _| ws.chats[ws.active].attachments.is_empty()));
    assert_eq!(
        workspace.read_with(cx, |ws, _| ws.send_queue.queued(ws.chats[ws.active].id)[0].attachments.clone()),
        vec![gpui_kit::SharedString::from("/tmp/b.rs")]
    );

    // A third attachment belongs to the composer, not the queued message.
    attach(cx, "/tmp/c.rs");
    until(&workspace, cx, |ws| user_msgs(ws, "second") == 1);
    let (text, atts) = workspace.read_with(cx, |ws, _| {
        let m = ws.chats[ws.active]
            .messages
            .iter()
            .find(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains("second")))
            .unwrap();
        (
            match &m.kind {
                MessageKind::Text(t) => t.to_string(),
                _ => String::new(),
            },
            m.attachments.clone(),
        )
    });
    assert!(text.contains("/tmp/b.rs") && !text.contains("/tmp/c.rs"), "queued prompt must carry only its snapshot");
    assert_eq!(atts.len(), 1);
    assert_eq!(&*atts[0], "/tmp/b.rs");
    assert_eq!(
        workspace.read_with(cx, |ws, _| ws.chats[ws.active].attachments.clone()),
        vec![gpui_kit::SharedString::from("/tmp/c.rs")],
        "the newer attachment stays live for the next message"
    );
}
