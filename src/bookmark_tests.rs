//! Tests for message bookmarks: the ⋯/footer star toggles `bookmarked` on
//! the message, the flag round-trips through `save_chats`/`load_chats`, the
//! chat ⋯ menu's Bookmarks submenu lists starred rows and scrolls to them,
//! and a regenerate drops the bookmark with its message.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-bookmark-test-{}", std::process::id()));
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
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

fn bookmarked(ws: &Entity<Workspace>, cx: &VisualTestContext) -> Vec<bool> {
    ws.read_with(cx, |ws, _| ws.chats[ws.active].messages.iter().map(|m| m.bookmarked).collect())
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
fn toggle_marks_and_unmarks_the_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "question");
    push(&ws, cx, Role::Assistant, "answer");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_bookmark(1, cx));
    });
    assert_eq!(bookmarked(&ws, cx), [false, true], "toggle stars the message");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_bookmark(1, cx));
    });
    assert_eq!(bookmarked(&ws, cx), [false, false], "a second toggle unstars it");
    // Out-of-range is a no-op, not a panic.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_bookmark(9, cx));
    });
    assert_eq!(bookmarked(&ws, cx), [false, false]);
}

#[test]
fn bookmark_survives_save_and_load() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "question");
    push(&ws, cx, Role::Assistant, "answer worth keeping");
    let dir = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.toggle_bookmark(1, cx); // toggle_bookmark saves
            this.project.chats_dir()
        })
    });
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].messages[0].bookmarked);
    assert!(loaded[0].messages[1].bookmarked, "the star round-trips through disk");

    // Toggling off persists too — the file must not keep a stale star.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_bookmark(1, cx));
    });
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    assert!(loaded[0].messages.iter().all(|m| !m.bookmarked), "unstarring persists");
}

/// The chat ⋯ menu's Bookmarks submenu lists starred messages numbered and
/// clipped; clicking one scrolls the transcript to that message.
#[test]
fn bookmarks_submenu_lists_and_jumps() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "the pinned question");
    for i in 0..40 {
        push(&ws, cx, Role::Assistant, &format!("filler reply {i}"));
    }
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.toggle_bookmark(0, cx));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The tail is followed, so message 0 is scrolled out of view.
        assert!(window.try_find(("msg", 0usize)).is_none_or(|s| !s.visible()), "message 0 starts off-screen");
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Bookmarks"))
            .unwrap_or_else(|| panic!("chat menu should offer Bookmarks"))
            .clone();
        window.within("popup-menu").hover(item.path().last().unwrap().clone(), cx);
        window.draw(cx).clear(cx);
        let labels: Vec<String> = snapshots(window)
            .iter()
            .filter(|s| s.path().iter().any(|id| *id == gpui_kit::ElementId::from("submenu")))
            .filter_map(|s| s.label().map(str::to_string))
            .collect();
        assert_eq!(labels, ["1. the pinned question"], "the submenu lists the starred message");
        window.within("submenu").click(0usize, cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("msg", 0usize)).visible(), "clicking a bookmark scrolls it into view");
    });
}

/// With nothing starred the submenu still renders — a disabled
/// "No bookmarks" row instead of an empty popover.
#[test]
fn bookmarks_submenu_empty_state() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Bookmarks"))
            .unwrap_or_else(|| panic!("chat menu should offer Bookmarks"))
            .clone();
        window.within("popup-menu").hover(item.path().last().unwrap().clone(), cx);
        window.draw(cx).clear(cx);
        let labels: Vec<String> = snapshots(window)
            .iter()
            .filter(|s| s.path().iter().any(|id| *id == gpui_kit::ElementId::from("submenu")))
            .filter_map(|s| s.label().map(str::to_string))
            .collect();
        assert_eq!(labels, ["No bookmarks"], "the empty submenu says so");
    });
}

/// Retrying the last turn drops the bookmarked reply with it — the flag
/// lives on the message, so nothing dangles.
#[test]
fn regenerate_drops_the_bookmark_with_the_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // A temp workdir keeps the turn checkpoint off the real repo.
    let workdir = std::env::temp_dir().join(format!("rixlcode-bookmark-retry-{}", std::process::id()));
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
            this.toggle_bookmark(1, cx);
            this.retry_last(cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.chats[ws.active].messages.len(), 1, "the reply was dropped for the retry");
        assert!(ws.chats[ws.active].messages.iter().all(|m| !m.bookmarked), "no bookmark survives the dropped message");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The footer's star affordance: hidden until hover on a plain row, pinned
/// visible once the message is starred.
#[test]
fn footer_star_marks_the_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "answer");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!window.find(("bookmark", 0usize)).visible(), "the star hides until hover");
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("bookmark", 0usize)).visible(), "hover reveals the star");
        window.click(("bookmark", 0usize), cx);
        window.draw(cx).clear(cx);
    });
    assert_eq!(bookmarked(&ws, cx), [true], "clicking the star bookmarks the message");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("bookmark", 0usize)).visible(), "a starred row keeps the icon pinned");
    });
}
