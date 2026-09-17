//! Headless tests for the failed-turn retry: the error row's Retry button
//! and "Retry turn" menu item re-send the turn's prompt through
//! `regenerate_from`, the failed attempt lands in the new reply's
//! `alternatives`, the button is disabled while a turn runs, and a
//! non-error row keeps the ghost icon instead of the button. Declared in
//! `views::mod` — `main.rs` is at the SLOC cap.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext, Window};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-retry-failed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// A temp workdir for the chat's turns — keeps checkpoint snapshots off
/// the real repo.
fn temp_workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-retry-failed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The first turn fails with an `Error` event; later sends record their
/// prompt, emit a text reply, then hang so the retried turn stays
/// observable mid-flight.
struct FlakyBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
    failed: std::sync::atomic::AtomicBool,
}

impl AgentBackend for FlakyBackend {
    fn name(&self) -> &'static str {
        "flaky"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        self.sent.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        if !self.failed.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let _ = tx.send(AgentEvent::Error("boom".into()));
        } else {
            let _ = tx.send(AgentEvent::TextStart);
            let _ = tx.send(AgentEvent::TextDelta("recovered".into()));
            std::mem::forget(tx); // producer never exits — the turn stays running
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Point the chat's turns at a temp workdir on the flaky backend and send
/// `prompt` — the first turn fails, leaving an error row.
fn fail_first_turn(
    ws: &Entity<Workspace>, cx: &mut VisualTestContext, workdir: &std::path::Path, prompt: &str,
) -> std::sync::Arc<parking_lot::Mutex<Vec<String>>> {
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sent2 = sent.clone();
    ws.update(cx, |this, _| {
        this.backend = std::sync::Arc::new(FlakyBackend { sent: sent2, failed: false.into() });
        this.chats[this.active].workdir = workdir.to_string_lossy().into_owned();
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.send_text(Queued::new(prompt.to_string(), Vec::new()), window, cx));
    });
    cx.run_until_parked();
    sent
}

/// Push a message without starting a reply.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// Menu-item labels in the snapshot tree.
fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window)
        .iter()
        .filter(|s| s.role() == Some(A11yRole::MenuItem))
        .filter_map(|s| s.label().map(str::to_string))
        .collect()
}

/// Click the top-level menu item with `label` — panics when it isn't
/// offered.
fn click_menu_item(window: &mut Window, label: &str, cx: &mut App) {
    let item = snapshots(window)
        .iter()
        .find(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some(label))
        .unwrap_or_else(|| panic!("menu should offer {label}"))
        .clone();
    let id = item.path().last().unwrap().clone();
    window.within("popup-menu").click(id, cx);
}

/// The error row's Retry button re-sends the same prompt; the retried
/// turn's reply replaces the error row.
#[test]
fn retry_button_resends_the_failed_prompt() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("button");
    let sent = fail_first_turn(&ws, cx, &workdir, "fix the bug");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("retry", 1usize)).visible(), "the error row shows a Retry button");
        window.click(("retry", 1usize), cx);
    });
    cx.run_until_parked();
    assert_eq!(sent.lock().as_slice(), ["fix the bug", "fix the bug"], "the same prompt was re-sent");
    app.read(|cx| {
        let chat = &ws.read(cx).chats[0];
        assert_eq!(chat.messages.len(), 2, "the error row was replaced by the new attempt");
        assert!(matches!(&chat.messages[1].kind, MessageKind::Text(t) if t.as_str() == "recovered"), "the retried turn's reply is live");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The failed attempt isn't lost — `regenerate_from` parks it in the new
/// reply's `alternatives`.
#[test]
fn retry_preserves_the_failed_attempt_as_an_alternative() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("alts");
    fail_first_turn(&ws, cx, &workdir, "fix the bug");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("retry", 1usize), cx);
    });
    cx.run_until_parked();
    app.read(|cx| {
        let reply = &ws.read(cx).chats[0].messages[1];
        assert_eq!(reply.alternatives.len(), 1, "the failed attempt joined the version chain");
        assert!(
            matches!(&reply.alternatives[0].kind, MessageKind::Text(t) if t.starts_with("**Error:**")),
            "the parked alternative is the error row"
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

/// While a turn runs the Retry button stays rendered but disabled — a
/// click can't start a second turn.
#[test]
fn retry_button_disabled_while_running() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("running");
    let sent = fail_first_turn(&ws, cx, &workdir, "fix the bug");
    ws.update(cx, |this, _| this.chats[this.active].running = true);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("retry", 1usize)).visible(), "the button stays rendered while running");
        window.click(("retry", 1usize), cx);
    });
    cx.run_until_parked();
    assert_eq!(sent.lock().len(), 1, "no second turn was sent");
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats[0].messages.len(), 2, "the transcript is untouched");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

/// A non-error assistant row keeps the hover-revealed ghost icon — no
/// always-visible Retry button.
#[test]
fn non_error_row_has_no_retry_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "hi");
    push(&ws, cx, Role::Assistant, "all good");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!window.find(("retry", 1usize)).visible(), "a plain reply keeps the ghost icon — invisible until hovered");
    });
}

/// The error row's menu offers "Retry turn" (not the plain "Retry" tail);
/// clicking it re-sends the prompt.
#[test]
fn error_row_menu_retries_the_turn() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("menu");
    let sent = fail_first_turn(&ws, cx, &workdir, "fix the bug");
    cx.update(|window, cx| {
        window.right_click(("msg", 1usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Retry turn"), "the error row offers Retry turn: {labels:?}");
        assert!(!labels.iter().any(|l| l == "Retry"), "the plain Retry tail is replaced: {labels:?}");
        click_menu_item(window, "Retry turn", cx);
    });
    cx.run_until_parked();
    assert_eq!(sent.lock().as_slice(), ["fix the bug", "fix the bug"], "the menu item re-sent the prompt");
    let _ = std::fs::remove_dir_all(&workdir);
}
