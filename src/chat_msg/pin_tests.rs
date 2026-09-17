//! Tests for message pinning: the ⋯/footer pin toggles `pinned` on the
//! message, one pin per chat (a second pin replaces the first), the flag
//! round-trips through `save_chats`/`load_chats`, the titlebar banner
//! jumps to the row and its × unpins, and a regenerate drops the pin with
//! its message.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-pin-test-{}", std::process::id()));
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

/// Push a text message without starting a turn, growing the scroller.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

fn pinned(ws: &Entity<Workspace>, cx: &VisualTestContext) -> Vec<bool> {
    ws.read_with(cx, |ws, _| ws.chats[ws.active].messages.iter().map(|m| m.pinned).collect())
}

/// A backend whose turn completes immediately — `retry_last` needs a real
/// `send` behind it, but no subprocess.
struct OkBackend;

impl AgentBackend for OkBackend {
    fn name(&self) -> &'static str {
        "ok"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta("done".into()));
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events: rx,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

#[test]
fn pin_toggles_and_a_second_pin_replaces_the_first() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "question");
    push(&ws, cx, Role::Assistant, "answer");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_message_pin(1, cx));
    });
    assert_eq!(pinned(&ws, cx), [false, true], "toggle pins the message");
    // One pin per chat — pinning elsewhere moves it.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_message_pin(0, cx));
    });
    assert_eq!(pinned(&ws, cx), [true, false], "the second pin replaces the first");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_message_pin(0, cx));
    });
    assert_eq!(pinned(&ws, cx), [false, false], "toggling the pinned row unpins it");
    // Out-of-range is a no-op, not a panic.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_message_pin(9, cx));
    });
    assert_eq!(pinned(&ws, cx), [false, false]);
}

#[test]
fn pin_survives_save_and_load() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "question");
    push(&ws, cx, Role::Assistant, "answer worth keeping");
    let dir = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.toggle_message_pin(1, cx); // toggle_message_pin saves
            this.project.chats_dir()
        })
    });
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].messages[0].pinned);
    assert!(loaded[0].messages[1].pinned, "the pin round-trips through disk");

    // Unpinning persists too — the file must not keep a stale pin.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.unpin_message(cx));
    });
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    assert!(loaded[0].messages.iter().all(|m| !m.pinned), "unpinning persists");
}

/// The banner under the titlebar: appears once a message is pinned, click
/// scrolls the transcript to it, and the × unpins (hiding the banner).
#[test]
fn banner_renders_jumps_and_unpins() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "the pinned question");
    for i in 0..40 {
        push(&ws, cx, Role::Assistant, &format!("filler reply {i}"));
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("pinned-banner").is_none(), "no pin, no banner");
    });
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_message_pin(0, cx));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let banner = window.find("pinned-banner");
        assert!(banner.visible(), "a pinned message raises the banner");
        assert_eq!(banner.label(), Some("Pinned: the pinned question"));
        // The tail is followed, so message 0 is scrolled out of view.
        assert!(window.try_find(("msg", 0usize)).is_none_or(|s| !s.visible()), "message 0 starts off-screen");
        window.click("pinned-banner", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("msg", 0usize)).visible(), "clicking the banner scrolls the pin into view");
        window.click("unpin", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("pinned-banner").is_none(), "the × hides the banner");
    });
    assert_eq!(pinned(&ws, cx), vec![false; 41], "the × cleared the pin");
}

/// Retrying the last turn drops the pinned reply with it — the flag lives
/// on the message, so nothing dangles.
#[test]
fn regenerate_drops_the_pin_with_the_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // A temp workdir keeps the turn checkpoint off the real repo.
    let workdir = std::env::temp_dir().join(format!("rixlcode-pin-retry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).unwrap();
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.backend = std::sync::Arc::new(OkBackend);
            this.chats[this.active].workdir = workdir.to_string_lossy().into_owned();
        });
    });
    push(&ws, cx, Role::User, "question");
    push(&ws, cx, Role::Assistant, "answer");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.toggle_message_pin(1, cx);
            this.retry_last(cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.chats[ws.active].messages.len(), 1, "the reply was dropped for the retry");
        assert!(ws.chats[ws.active].messages.iter().all(|m| !m.pinned), "no pin survives the dropped message");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The footer's pin affordance: hidden until hover on a plain row, kept
/// visible once the message is pinned.
#[test]
fn footer_pin_marks_the_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "answer");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!window.find(("pin", 0usize)).visible(), "the pin hides until hover");
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("pin", 0usize)).visible(), "hover reveals the pin");
        window.click(("pin", 0usize), cx);
        window.draw(cx).clear(cx);
    });
    assert_eq!(pinned(&ws, cx), [true], "clicking the pin pins the message");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("pin", 0usize)).visible(), "a pinned row keeps the icon visible");
    });
}
