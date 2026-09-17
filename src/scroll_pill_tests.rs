//! Tests for the "jump to latest" pill: the scroller's bottom-center
//! overlay appears when the transcript leaves the tail, counts messages
//! appended while scrolled up, and its click returns to the live edge and
//! marks the chat read. Declared as `crate::chat_search::scroll_pill_tests`
//! via `#[path]` so `main.rs` stays under the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// The built-in jump button's id inside the `chat-messages` scroller.
fn pill() -> (ElementId, &'static str) {
    (ElementId::from("chat-messages"), "jump-to-latest")
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-scroll-pill-test-{}", std::process::id()));
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

/// Append a text message, growing the scroller — `append` keeps the scroll
/// anchor, unlike `reset`, which would re-follow the tail.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            alternatives: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// Seed `n` messages in one shot — the transcript starts tail-followed.
fn seed(ws: &Entity<Workspace>, cx: &mut VisualTestContext, n: usize) {
    ws.update(cx, |this, cx| {
        let messages = std::rc::Rc::make_mut(&mut this.chats[this.active].messages);
        for i in 0..n {
            messages.push(ChatMessage {
                role: Role::User,
                kind: MessageKind::Text(format!("seed {i}").into()),
                rating: None,
                bookmarked: false,
                pinned: false,
                usage: None,
                attachments: vec![],
                alternatives: vec![],
                at: std::time::SystemTime::now(),
            });
        }
        this.scroller.update(cx, |s, cx| s.reset(n, cx));
        cx.notify();
    });
}

/// Scroll the transcript to its first row — the state the pill watches.
fn scroll_to_top(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        this.scroller.update(cx, |s, cx| s.scroll_to_item(0, cx));
    });
}

/// Let the pill's fade transition finish, then draw a frame.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[gpui_kit::test]
fn pill_hidden_at_bottom(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed(&ws, cx, 40);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(pill()).is_none(), "no pill while tail-followed");
    });
    ws.read_with(cx, |ws, _| assert_eq!(ws.pill_anchor, None));
}

#[gpui_kit::test]
fn pill_counts_new_messages(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed(&ws, cx, 40);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    scroll_to_top(&ws, cx);
    settle(cx);
    cx.update(|window, _cx| {
        let pill = window.find(pill());
        assert!(pill.visible(), "scrolled up shows the pill");
        assert_eq!(pill.label(), Some("Latest"), "nothing new yet");
    });
    ws.read_with(cx, |ws, _| assert_eq!(ws.pill_anchor, Some(40)));

    push(&ws, cx, "one");
    push(&ws, cx, "two");
    push(&ws, cx, "three");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find(pill()).label(), Some("3 new"), "arrivals while scrolled up count");
    });
}

#[gpui_kit::test]
fn pill_click_jumps_and_marks_read(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed(&ws, cx, 40);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    scroll_to_top(&ws, cx);
    settle(cx);
    push(&ws, cx, "fresh");
    ws.update(cx, |this, _| this.chats[this.active].unread = true);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find(pill()).label(), Some("1 new"));
        window.click(pill(), cx);
    });
    settle(cx);
    cx.update(|window, _cx| {
        assert!(window.try_find(pill()).is_none(), "click hides the pill at the tail");
    });
    ws.read_with(cx, |ws, cx| {
        assert!(ws.scroller.read(cx).is_following_tail(), "click resumes tail-following");
        assert!(!ws.chats[ws.active].unread, "click marks the chat read");
        assert_eq!(ws.pill_anchor, None);
    });
}

#[gpui_kit::test]
fn pill_hides_when_scrolled_back(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed(&ws, cx, 40);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    scroll_to_top(&ws, cx);
    settle(cx);
    cx.update(|window, _cx| assert!(window.find(pill()).visible()));
    ws.update(cx, |this, cx| {
        this.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
    });
    settle(cx);
    cx.update(|window, _cx| {
        assert!(window.try_find(pill()).is_none(), "returning to the tail hides the pill");
    });
    ws.read_with(cx, |ws, _| assert_eq!(ws.pill_anchor, None, "anchor cleared at the tail"));
}
