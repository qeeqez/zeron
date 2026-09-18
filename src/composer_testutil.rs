//! Shared headless-composer helpers: mount a `Workspace` in a test window and
//! drive it with real input events. Used by `composer_tests` and
//! `composer_queue_tests`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, point, px, size};

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
pub(crate) fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
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
    let workspace = workspace.unwrap();
    // The project-file scan runs on the background executor — poll until it
    // lands instead of assuming run_until_parked covers real threads.
    for _ in 0..200 {
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| !ws.project_files.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(workspace.read_with(cx, |ws, _| !ws.project_files.is_empty()), "project file scan never landed");
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |composer, cx| composer.focus(window, cx));
        });
    });
    (workspace, cx)
}

/// The composer's current text.
pub(crate) fn composer_value(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    workspace.read_with(cx, |ws, app| ws.composer.read(app).value().to_string())
}

/// Send `text` through the real input path: type, then Enter.
pub(crate) fn type_and_send(cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        window.input(text, cx);
        window.press("enter", cx);
    });
}

/// Advance the test clock until `cond` holds or the budget runs out.
pub(crate) fn until(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| cond(ws)) {
            return;
        }
    }
    panic!("condition never held");
}

/// Wait until a freshly opened dialog's slide-in animation is done. The
/// animation is wall-clock driven (`Instant::elapsed` per frame), so
/// `advance_clock` can't settle it — only real time does. Poll every
/// element's painted bounds across real-time-separated frames; once two
/// consecutive frames agree, nothing is still sliding and clicks hit the
/// rows' final positions instead of the backdrop behind them.
pub(crate) fn settle_dialog(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut prev = None;
    loop {
        let bounds: Vec<_> = cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            gpui_kit::base::test_support::snapshots(window).iter().map(|s| s.bounds()).collect()
        });
        if prev.as_ref() == Some(&bounds) {
            return;
        }
        prev = Some(bounds);
        assert!(std::time::Instant::now() < deadline, "dialog animation never settled");
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

/// Click the in-app toast near its bottom edge until `cond` observes the
/// effect. The slide-in is wall-clock driven like dialog animations, so a
/// lone click can land on a stale position mid-animation — poll instead.
pub(crate) fn click_toast_until(cx: &mut VisualTestContext, cond: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            if let Some(toast) = window.try_find("notification") {
                let y_inside = (toast.bounds().size.height - px(5.)).max(px(0.));
                window.click_at("notification", point(px(20.), y_inside), cx);
            }
        });
        cx.run_until_parked();
        if cond(cx) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the toast click never landed");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Count user messages whose text contains `needle`.
pub(crate) fn user_msgs(ws: &Workspace, needle: &str) -> usize {
    ws.chats[ws.active]
        .messages
        .iter()
        .filter(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains(needle)))
        .count()
}

/// Point the workspace at the sim backend so sends complete on the test clock.
pub(crate) fn use_sim(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(crate::backend::SimBackend);
        });
    });
}
