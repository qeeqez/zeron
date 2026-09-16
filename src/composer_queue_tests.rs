//! Headless tests for the send queue: per-workspace scoping, slash commands
//! deferred behind a running reply, and per-message attachment snapshots.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, VisualTestContext, px, size};

use crate::composer_testutil::{open_workspace, type_and_send, until, use_sim, user_msgs};
use crate::model::{Chat, MessageKind, Role};
use crate::send_queue::{Queued, SendQueue};
use crate::workspace::Workspace;

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

/// `/compact` submitted mid-stream must queue like text: the compaction
/// turn can't race the reply it would fold.
#[gpui_kit::test]
fn slash_compact_queued_mid_stream(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "first");

    until(&workspace, cx, |ws| ws.chats[ws.active].running);
    type_and_send(cx, "/compact");

    // Deferred: no compact user row while the reply is in flight.
    assert!(workspace.read_with(cx, |ws, _| {
        !ws.chats[ws.active]
            .messages
            .iter()
            .any(|m| matches!(&m.kind, MessageKind::Text(t) if t == "/compact"))
    }));
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("queued-0").is_some(), "/compact should sit in the queue");
    });

    // Turn ends → the queued command runs as its own turn.
    until(&workspace, cx, |ws| {
        !ws.chats[ws.active].running
            && ws.chats[ws.active]
                .messages
                .iter()
                .any(|m| matches!(&m.kind, MessageKind::Text(t) if t == "/compact"))
    });
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

/// Editing a queued message keeps its slot: the item leaves the queue while
/// parked, then commits back at its original index with the new text.
#[test]
fn queue_edit_updates_in_place() {
    let mut q = SendQueue::default();
    let live = |_| true;
    q.enqueue(7, Queued::new("aaa".into(), vec![]), live);
    q.enqueue(7, Queued::new("bbb".into(), vec![]), live);
    q.enqueue(7, Queued::new("ccc".into(), vec![]), live);

    let parked = q.begin_edit(7, 1, "draft".into(), vec![]).expect("id 1 is queued");
    assert_eq!(parked.text, "bbb");
    assert!(q.editing_for(7));
    assert_eq!(q.queued(7).iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["aaa", "ccc"]);

    let edit = q.commit_edit("bbb edited".into(), vec![]).expect("edit is open");
    assert_eq!(edit.saved_text, "draft");
    assert!(!q.editing_for(7));
    assert_eq!(q.queued(7).iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["aaa", "bbb edited", "ccc"]);
}

/// An empty commit cancels the edit — the original message returns to its
/// slot unchanged (deletion is the row's ✕, not a blank send).
#[test]
fn queue_edit_empty_commit_restores() {
    let mut q = SendQueue::default();
    let live = |_| true;
    q.enqueue(7, Queued::new("aaa".into(), vec![]), live);
    q.enqueue(7, Queued::new("bbb".into(), vec![]), live);

    q.begin_edit(7, 1, String::new(), vec![]);
    q.commit_edit("   ".into(), vec![]);
    assert_eq!(q.queued(7).iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["aaa", "bbb"]);
}

/// Reorder changes send order: up/down move an item, edges are no-ops, and
/// send-now jumps it to the front.
#[test]
fn queue_reorder_and_send_now() {
    let mut q = SendQueue::default();
    let live = |_| true;
    for text in ["a", "b", "c"] {
        q.enqueue(7, Queued::new(text.into(), vec![]), live);
    }
    let order = |q: &SendQueue| q.queued(7).iter().map(|i| i.text.clone()).collect::<Vec<_>>();

    assert!(!q.move_by(7, 0, -1), "first item can't move up");
    assert!(q.move_by(7, 1, -1), "b moves up");
    assert_eq!(order(&q), ["b", "a", "c"]);
    assert!(q.move_by(7, 1, 1), "b moves back down");
    assert_eq!(order(&q), ["a", "b", "c"]);
    assert!(!q.move_by(7, 2, 1), "c is already last — no-op");
    assert_eq!(order(&q), ["a", "b", "c"]);
    assert!(q.move_to_front(7, 2));
    assert_eq!(order(&q), ["c", "a", "b"]);
    assert!(!q.move_by(7, 99, -1), "unknown id is a no-op");
}

/// The queue persists per chat: a fresh SendQueue over the same directory
/// adopts the pending messages in order, keyed by `created_at` so chat id
/// reassignment on load can't misroute them.
#[test]
fn queue_persists_per_chat() {
    let dir = std::env::temp_dir().join(format!("rixlcode-queue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut chats = vec![Chat::new(0, "one"), Chat::new(1, "two")];
    let mut q = SendQueue::default();
    let live = |id| chats.iter().any(|c| c.id == id);
    q.enqueue(0, Queued::new("first".into(), vec![]), live);
    q.enqueue(0, Queued::new("second".into(), vec![]), live);
    q.enqueue(1, Queued::new("other chat".into(), vec![]), live);
    q.persist(&dir, &chats);

    // Reloaded chats get fresh ids — the queue must still find them.
    let mut reloaded = vec![Chat::new(5, "one"), Chat::new(6, "two")];
    reloaded[0].created_at = chats[0].created_at;
    reloaded[1].created_at = chats[1].created_at;
    let mut restored = SendQueue::default();
    restored.hydrate(&dir, &reloaded);
    assert_eq!(restored.queued(5).iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    assert_eq!(restored.queued(6).iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["other chat"]);

    // Deleting a chat drops its queue from the file on the next persist.
    chats.remove(0);
    q.persist(&dir, &chats);
    let mut after_delete = SendQueue::default();
    after_delete.hydrate(&dir, &reloaded);
    assert!(after_delete.queued(5).is_empty(), "deleted chat's queue must not persist");
    assert_eq!(after_delete.queued(6).len(), 1);
}
