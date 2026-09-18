//! Headless tests for the failed-toast Retry button: clicking it dismisses
//! the toast, surfaces the failed chat, and re-sends its last prompt
//! through the live backend. Lives in its own file — `notify_tests.rs` is
//! at the SLOC cap. Declared in `main.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, point, px, size};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

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
    (workspace.unwrap(), cx)
}

/// Records every prompt it is asked to run, replies "retried", then hangs
/// — the producer never exits, so the retried turn stays running and no
/// completion toast replaces the one the button was supposed to dismiss.
struct RecordingBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        self.sent.lock().push(prompt.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta("retried".into()));
        std::mem::forget(tx);
        ReplyStream {
            events: rx,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

fn text(role: Role, body: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind: MessageKind::Text(body.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

/// Seed a failed turn (user prompt + error reply + flag) against the
/// recording backend and return the chat id the toast keys on.
fn fail_turn(ws: &Entity<Workspace>, sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>, cx: &mut VisualTestContext) -> u64 {
    ws.update(cx, |this, _| {
        this.backend = std::sync::Arc::new(RecordingBackend { sent });
        this.notify_on_done = true;
        this.notify_background = false;
        this.notify_sound = false;
        let chat = &mut this.chats[0];
        let msgs = std::rc::Rc::make_mut(&mut chat.messages);
        msgs.push(text(Role::User, "fix the bug"));
        msgs.push(text(Role::Assistant, "**Error:** codex exited 1"));
        chat.failed_flag = true;
        chat.id
    })
}

/// In-app toasts mounted under the Root's notification layer.
fn toast_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| {
        let Some(Some(root)) = window.root::<Root>() else { return 0 };
        root.read(cx).notification.read(cx).notifications().len()
    })
}

#[gpui_kit::test]
fn retry_button_dismisses_toast_and_resends(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let chat_id = fail_turn(&ws, sent.clone(), cx);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.notify_done(chat_id, window, cx)));
    // The toast may still be sliding in when the first click lands, so the
    // observable send — not the click — is the success signal, matching
    // the in-chat retry tests. 10s budget.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            if window.find(("reply-retry", chat_id)).visible() {
                window.click(("reply-retry", chat_id), cx);
            }
        });
        cx.run_until_parked();
        if !sent.lock().is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the toast Retry click never re-sent the prompt");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // The hanging backend keeps the retried turn open, so no completion
    // toast can replace the failed one — and no title pass runs.
    assert_eq!(sent.lock().as_slice(), ["fix the bug"], "Retry re-sent the failed prompt");
    // The delta lands through the pump on the next clock tick.
    for _ in 0..32 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if ws.read_with(
            cx,
            |w, _| matches!(w.chats[0].messages.last().map(|m| &m.kind), Some(MessageKind::Text(t)) if t.as_str() == "retried"),
        ) {
            break;
        }
    }
    ws.read_with(cx, |this, _| {
        let chat = &this.chats[0];
        assert!(chat.running, "the retried turn is mid-flight");
        assert!(!chat.failed_flag, "the retried turn cleared the failure");
        assert!(matches!(chat.messages.last().map(|m| &m.kind), Some(MessageKind::Text(t)) if t.as_str() == "retried"));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(toast_count(cx), 0, "the Retry click should close its own toast");
}

/// A turn parked on an approval request posts a toast; clicking it opens
/// the waiting chat.
#[gpui_kit::test]
fn approval_request_toast_opens_the_chat(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let chat_id = ws.update(cx, |this, _| {
        this.notify_on_done = true;
        this.notify_background = false;
        this.notify_sound = false;
        this.chats[0].id
    });
    let (respond, _decisions) = std::sync::mpsc::channel();
    ws.update(cx, |this, cx| {
        this.apply_events(
            chat_id,
            vec![AgentEvent::ApprovalRequest {
                ix: 0,
                kind: crate::backend::ApprovalKind::Command,
                detail: "rm -rf ./build".into(),
                respond,
            }],
            cx,
        );
    });
    // The notify path defers through `spawn` → `update_in`.
    cx.run_until_parked();
    assert_eq!(toast_count(cx), 1, "a pending approval should post a toast");
    ws.update(cx, |this, cx| this.new_chat(cx));
    assert_eq!(ws.read_with(cx, |w, _| w.active), 1);
    // Same shape as the done-toast click: the toast slides in from the top
    // edge, so click near its bottom where it's already on-screen.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let b = window.find("notification").bounds();
        let y_inside = (b.size.height - px(5.)).max(px(0.));
        window.click_at("notification", point(px(20.), y_inside), cx);
    });
    cx.run_until_parked();
    assert_eq!(ws.read_with(cx, |w, _| w.active), 0, "clicking the approval toast should open its chat");
}

/// `notify_on_done` gates approval notices too — the toggle is the
/// user-visible "toast/system surfaces" switch, not strictly "reply done".
#[gpui_kit::test]
fn approval_notice_respects_notify_toggle(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let chat_id = ws.update(cx, |this, _| {
        this.notify_on_done = false;
        this.chats[0].id
    });
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.notify_approval(chat_id, "Apply patch: apply diff", window, cx));
    });
    assert_eq!(toast_count(cx), 0, "notify_on_done off must silence approval notices");
}
