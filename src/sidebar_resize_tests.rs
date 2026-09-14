//! Headless UI tests for the sidebar resize handle: the drag must track the
//! cursor 1:1 (the `Sidebar` component's Offcanvas width transition used to
//! restart on every mouse-move, so the edge chased the cursor and the view
//! read as shifting left/right), clamp to the width bounds, and the handle
//! must stay out of the titlebar's window-drag strip. Mount pattern matches
//! `ui_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, point, px};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-resize-test-{}", std::process::id()));
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

/// Dragging the resize handle sets `sidebar_width` to the cursor's x,
/// clamped to [SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX].
#[test]
fn resize_drag_tracks_cursor_and_clamps() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let handle = window.find("sidebar-resize");
        assert!(handle.visible(), "resize handle should render");
        let grab = handle.bounds().center();

        // Drag right: width follows the cursor's x exactly.
        window.drag(grab, point(px(340.), grab.y), cx);
        assert_eq!(ws.read(cx).sidebar_width, 340., "width must equal cursor x");
        assert!(!ws.read(cx).resizing_sidebar, "mouse-up ends the drag");

        // Drag past the max: clamps, doesn't follow the cursor out of range.
        let handle = window.find("sidebar-resize");
        window.drag(handle.bounds().center(), point(px(900.), grab.y), cx);
        assert_eq!(ws.read(cx).sidebar_width, crate::window::SIDEBAR_WIDTH_MAX);

        // Drag past the min: clamps at the floor.
        let handle = window.find("sidebar-resize");
        window.drag(handle.bounds().center(), point(px(20.), grab.y), cx);
        assert_eq!(ws.read(cx).sidebar_width, crate::window::SIDEBAR_WIDTH_MIN);
    });
}

/// The rendered sidebar column must be exactly `sidebar_width` on the very
/// next frame — no width transition chasing the cursor. Pre-fix the
/// Offcanvas wrapper animated from the previous target, so the first frame
/// after a width change rendered the OLD width.
#[test]
fn sidebar_width_applies_without_animation() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        ws.update(cx, |ws, cx| {
            ws.sidebar_width = 320.;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        let wrap = window.find("sidebar-wrap").bounds();
        let width = f32::from(wrap.size.width);
        assert!((width - 321.).abs() < 0.5, "sidebar-wrap must render at sidebar_width + 1px border on the first frame, got {width}");
    });
}

/// The resize handle starts below the titlebar drag strip — the top
/// TOP_BAR_H of the sidebar is a window-move zone, so a handle overlapping
/// it would fight window drags.
#[test]
fn resize_handle_stays_below_titlebar_strip() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let handle = window.find("sidebar-resize").bounds();
        assert!(
            handle.origin.y >= px(crate::window::TOP_BAR_H),
            "handle top {} must sit below the {}px window-drag strip",
            handle.origin.y,
            crate::window::TOP_BAR_H
        );
    });
}
