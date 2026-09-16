//! Tests for `Workspace::regenerate_from`: mid-chat regenerates confirm
//! first, then drop the reply and everything after it and re-send the last
//! user prompt; regenerating the last message skips the confirm; a running
//! chat ignores the call. Declared from `chat_delete.rs` via `#[path]` —
//! `main.rs` is at the SLOC cap.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// A backend that records each prompt, then never produces an event — the
/// turn stays `running` so tests can observe the in-flight state.
struct HungBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for HungBackend {
    fn name(&self) -> &'static str {
        "hung"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        self.sent.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        std::mem::forget(tx); // producer never exits — the pump blocks on recv
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-regen-test-{}", std::process::id()));
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

/// Point the chat's turns at a temp workdir on the hung backend — keeps
/// checkpoint snapshots out of the real repo.
fn use_hung_backend(
    ws: &Entity<Workspace>, cx: &mut VisualTestContext, workdir: &std::path::Path,
) -> std::sync::Arc<parking_lot::Mutex<Vec<String>>> {
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    ws.update(cx, |this, _| {
        this.backend = std::sync::Arc::new(HungBackend { sent: sent.clone() });
        this.chats[this.active].workdir = workdir.to_string_lossy().into_owned();
    });
    sent
}

/// A temp workdir for the chat's turns.
fn temp_workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-regen-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Push a message without starting a reply.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
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

/// Seed a two-turn transcript: user/assistant/user/assistant.
fn seed_two_turns(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    push(ws, cx, Role::User, "first");
    push(ws, cx, Role::Assistant, "reply one");
    push(ws, cx, Role::User, "second");
    push(ws, cx, Role::Assistant, "reply two");
}

#[test]
fn regenerate_mid_chat_truncates_and_resends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("mid");
    let sent = use_hung_backend(&ws, cx, &workdir);
    seed_two_turns(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    // `cx`'s borrow of `app` ends here — the prompt helpers live on `app`.
    assert!(app.has_pending_prompt(), "mid-chat regenerate asks for confirmation");
    app.simulate_prompt_answer("Regenerate");
    app.run_until_parked();
    app.read(|cx| {
        let chat = &ws.read(cx).chats[0];
        assert_eq!(chat.messages.len(), 1, "the reply and everything after it are dropped");
        assert!(matches!(chat.messages[0].role, Role::User));
        assert!(chat.running, "the resend started a fresh turn");
        assert!(chat.started_at.is_some());
        assert!(!chat.failed_flag);
    });
    assert_eq!(sent.lock().as_slice(), ["first"], "the last user prompt before ix was re-sent");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn regenerate_last_message_needs_no_confirm() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("last");
    let sent = use_hung_backend(&ws, cx, &workdir);
    seed_two_turns(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(3, window, cx));
    });
    assert!(!app.has_pending_prompt(), "nothing is lost past the last message — no confirm");
    app.run_until_parked();
    app.read(|cx| {
        let chat = &ws.read(cx).chats[0];
        assert_eq!(chat.messages.len(), 3, "only the last reply is dropped");
        assert!(chat.running);
    });
    assert_eq!(sent.lock().as_slice(), ["second"]);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn cancel_leaves_the_transcript_untouched() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("cancel");
    let sent = use_hung_backend(&ws, cx, &workdir);
    seed_two_turns(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    assert!(app.has_pending_prompt());
    app.simulate_prompt_answer("Cancel");
    app.run_until_parked();
    app.read(|cx| {
        let chat = &ws.read(cx).chats[0];
        assert_eq!(chat.messages.len(), 4, "cancel keeps every message");
        assert!(!chat.running, "no turn started");
    });
    assert!(sent.lock().is_empty(), "nothing was re-sent");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn running_chat_ignores_regenerate() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("running");
    let sent = use_hung_backend(&ws, cx, &workdir);
    seed_two_turns(&ws, cx);
    ws.update(cx, |this, _| this.chats[this.active].running = true);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    assert!(!app.has_pending_prompt(), "a running chat never reaches the confirm");
    app.run_until_parked();
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats[0].messages.len(), 4, "the transcript is untouched");
    });
    assert!(sent.lock().is_empty());
    let _ = std::fs::remove_dir_all(&workdir);
}

/// `retry_last` still drops the trailing reply and re-sends the last user
/// prompt — the shared `rerun_last_prompt` path.
#[test]
fn retry_last_still_resends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("retry");
    let sent = use_hung_backend(&ws, cx, &workdir);
    seed_two_turns(&ws, cx);
    ws.update(cx, |this, cx| this.retry_last(cx));
    app.run_until_parked();
    app.read(|cx| {
        let chat = &ws.read(cx).chats[0];
        assert_eq!(chat.messages.len(), 3, "the trailing reply is popped");
        assert!(chat.running);
    });
    assert_eq!(sent.lock().as_slice(), ["second"]);
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The hover toolbar's retry icon now sits on every assistant row; on a
/// mid-chat reply it confirms, then truncates and re-sends.
#[test]
fn retry_icon_regenerates_a_mid_chat_reply() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("icon");
    let sent = use_hung_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    push(&ws, cx, Role::User, "second");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 1usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("retry", 1usize)).visible(), "the retry icon shows on a mid-chat reply");
        window.click(("retry", 1usize), cx);
    });
    assert!(app.has_pending_prompt(), "the icon's regenerate confirms first");
    app.simulate_prompt_answer("Regenerate");
    app.run_until_parked();
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats[0].messages.len(), 1, "the reply and later messages are dropped");
    });
    assert_eq!(sent.lock().as_slice(), ["first"]);
    let _ = std::fs::remove_dir_all(&workdir);
}
