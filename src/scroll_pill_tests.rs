//! Tests for the "jump to latest" pill: the scroller's bottom-center
//! overlay appears when the transcript leaves the tail, counts messages
//! appended while scrolled up, and its click returns to the live edge and
//! marks the chat read. Declared as `crate::chat_search::scroll_pill_tests`
//! via `#[path]` so `main.rs` stays under the SLOC cap.
//!
//! Detach is sticky: scrolling up pauses tail-follow and nothing re-engages
//! it but the user — wheeling back to the bottom, clicking the pill, or a
//! `reset` (chat switch, send, search). A turn ending does not reattach;
//! the transcript stays where the user left it across turns.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, ScrollDelta, TestAppContext, VisualTestContext, point, px};

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

/// The topmost fully visible message row's index — the view's scroll anchor.
/// Rows clipped at the viewport edge aren't `visible`, so they can't be
/// wheel targets either.
fn top_row_ix(window: &gpui_kit::Window) -> usize {
    gpui_kit::base::test_support::snapshots(window)
        .iter()
        .filter(|s| s.visible())
        .filter_map(|s| match s.path().last() {
            Some(ElementId::NamedInteger(name, ix)) if name.as_ref() == "msg" => Some((*ix as usize, s.bounds().origin.y)),
            _ => None,
        })
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(ix, _)| ix)
        .expect("a message row is rendered")
}

/// Wheel-scroll the transcript — the real user path through the scroller's
/// scroll mask, unlike `scroll_to_item`/`scroll_to_end`.
fn wheel(row: usize, delta_y: f32, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.scroll(("msg", row), ScrollDelta::Pixels(point(px(0.), px(delta_y))), cx);
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

#[gpui_kit::test]
fn scroll_up_detaches_follow(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed(&ws, cx, 40);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    wheel(39, 300., cx);
    settle(cx);
    ws.read_with(cx, |ws, cx| {
        let s = ws.scroller.read(cx);
        assert!(!s.is_following_tail(), "wheeling up detaches tail-follow");
        assert!(s.is_scrolled_up());
    });
    let top_before = cx.update(|window, _cx| top_row_ix(window));
    push(&ws, cx, "streamed while scrolled up");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(top_row_ix(window), top_before, "new content must not move the detached view");
        assert_eq!(window.find(pill()).label(), Some("1 new"), "the pill counts the arrival");
    });
    ws.read_with(cx, |ws, cx| assert!(!ws.scroller.read(cx).is_following_tail()));
}

#[gpui_kit::test]
fn wheel_to_bottom_reattaches(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed(&ws, cx, 40);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    scroll_to_top(&ws, cx);
    settle(cx);
    push(&ws, cx, "fresh");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(pill()).visible(), "detached with new content shows the pill");
    });
    // Wheel down on the topmost row — the pill floats over the bottom rows,
    // so a wheel dispatched there would hit the pill's hitbox, not the
    // scroller's mask. The mask clamps deltas short of the list's bottom
    // padding, so the gesture that reaches the edge is the one whose event
    // bubbles to the list — repeat like a real overscroll.
    for _ in 0..3 {
        let top = cx.update(|window, _cx| top_row_ix(window));
        wheel(top, -5000., cx);
    }
    settle(cx);
    ws.read_with(cx, |ws, cx| {
        assert!(ws.scroller.read(cx).is_following_tail(), "wheeling to the bottom reattaches follow");
        assert_eq!(ws.pill_anchor, None);
    });
    cx.update(|window, _cx| {
        assert!(window.try_find(pill()).is_none(), "reattach hides the pill");
    });
}
