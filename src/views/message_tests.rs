//! Headless tests for the message row: hover-revealed actions (copy,
//! retry, view-raw, rating, speak), the duration label, and retry
//! re-sending the last user prompt.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-msg-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

/// Seed the active chat with a completed assistant turn.
fn seed_reply(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        this.push_note("reply body".into(), cx);
        this.chats[this.active].last_turn = Some(std::time::Duration::from_secs(7));
    });
}

/// Push a user message without starting a reply — retry tests seed the
/// prompt directly instead of going through `send`.
fn seed_user(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(text.into()),
            rating: None,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

#[test]
fn assistant_actions_reveal_on_hover() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_reply(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Ghost icons exist but stay invisible until the row is hovered.
        assert!(!window.find(("copy", 0usize)).visible(), "copy hidden before hover");
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        for id in ["copy", "raw", "retry", "up", "down", "speak"] {
            assert!(window.find((id, 0usize)).visible(), "{id} should reveal on hover");
        }
    });
}

#[test]
fn completed_turn_shows_duration_and_feedback_toggles() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_reply(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("worked", 0usize)).visible(), "duration label should show");
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        window.click(("up", 0usize), cx);
        assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(true));
        window.draw(cx).clear(cx);
        window.click(("down", 0usize), cx);
        assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(false));
    });
    // The seeded message is an assistant note — verify role for sanity.
    app.read(|cx| assert!(matches!(ws.read(cx).chats[0].messages[0].role, Role::Assistant)));
}
/// Records every prompt the workspace sends so retry can be asserted
/// against the real `run_backend` path. The stream ends immediately.
struct RecordingBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        self.sent.lock().push(prompt.to_string());
        let (_tx, events) = std::sync::mpsc::channel();
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// The hover toolbar's retry button drops the last assistant reply and
/// re-sends the last user prompt.
#[test]
fn retry_resends_last_user_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    ws.update(cx, |this, _cx| {
        this.backend = std::sync::Arc::new(RecordingBackend { sent: sent.clone() });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 1usize), cx);
        window.draw(cx).clear(cx);
        window.click(("retry", 1usize), cx);
    });
    let prompts = sent.lock().clone();
    assert_eq!(prompts, ["fix the bug"], "retry must re-send the last user prompt");
    app.read(|cx| {
        let msgs = &ws.read(cx).chats[0].messages;
        assert_eq!(msgs.len(), 1, "stale assistant reply should be popped");
        assert!(matches!(msgs[0].role, Role::User));
    });
}
