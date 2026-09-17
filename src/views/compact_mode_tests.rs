//! Headless tests for the Appearance section's "Compact messages" switch:
//! it writes `Workspace::compact_mode`, persists to settings.json, and the
//! transcript re-measures — rows, day separators and the gaps between them
//! all shrink.

use std::time::SystemTime;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-compact-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
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

/// Append a user message stamped `at` and grow the scroller — same shape as
/// the other transcript test helpers.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, s: &str, at: SystemTime) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::User,
            kind: MessageKind::Text(s.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at,
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

/// A fixed day in the past — always a day boundary against `now`.
fn old_day() -> SystemTime {
    SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 60 * 60)
}

/// Open Settings → Appearance and paint it.
fn open_appearance(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-appearance", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-appearance").visible());
    });
}

/// The Messages group's compact switch writes `Workspace::compact_mode` and
/// persists it to settings.json.
#[test]
fn compact_toggle_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert!(!ws.read_with(cx, |w, _| w.compact_mode), "compact defaults off");
    open_appearance(cx);
    cx.update(|window, cx| {
        let toggle = window.find("toggle-compact");
        assert_eq!(toggle.checked(), Some(false), "switch mirrors the flag");
        window.click("toggle-compact", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).compact_mode, "switch click should set the flag");
        assert!(crate::persist::load_settings().compact_mode, "switch click should persist");
        assert_eq!(window.find("toggle-compact").checked(), Some(true));
    });
}

/// Flipping compact on shrinks a rendered row and its day separator, and —
/// because the toggle re-measures the scroller — the next row's slot moves
/// up with it (a stale height cache would leave the stride unchanged even
/// though the row element itself shrank).
#[test]
fn compact_mode_shrinks_rows_and_remeasures() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, "first", old_day());
    push(&ws, cx, "second", SystemTime::now());
    push(&ws, cx, "third", SystemTime::now());
    let (h0, stride, sep_h) = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let row0 = window.find(("msg", 0usize)).bounds();
        let row1 = window.find(("msg", 1usize)).bounds();
        let sep = window.find(("date-separator", 1usize)).bounds();
        (f32::from(row0.size.height), f32::from(row1.origin.y - row0.origin.y), f32::from(sep.size.height))
    });
    open_appearance(cx);
    cx.update(|window, cx| {
        window.click("toggle-compact", cx);
        window.click("settings-back", cx);
        window.draw(cx).clear(cx);
    });
    let (h0_c, stride_c, sep_h_c) = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let row0 = window.find(("msg", 0usize)).bounds();
        let row1 = window.find(("msg", 1usize)).bounds();
        let sep = window.find(("date-separator", 1usize)).bounds();
        (f32::from(row0.size.height), f32::from(row1.origin.y - row0.origin.y), f32::from(sep.size.height))
    });
    assert!(h0_c < h0, "row height {h0_c} should shrink below {h0}");
    assert!(stride_c < stride, "row stride {stride_c} should shrink below {stride} — the scroller must re-measure");
    assert!(sep_h_c < sep_h, "separator height {sep_h_c} should shrink below {sep_h}");
}
