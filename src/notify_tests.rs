//! Headless UI tests for reply-completion notifications: drive a real
//! `Workspace` window through send → reply done and assert the platform
//! notification surface. Requires gpui-kit's `test-support` feature.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::backend::SimBackend;
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    sandbox_home();
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The test platform drops notifications posted without an identity,
        // matching the Linux/Windows behavior main.rs sets up.
        cx.set_app_identity("com.rixl.rixlcode", "Rixl Code");
    });
    let mut workspace = None;
    let window = cx.open_window(size(px(1024.), px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    (workspace.unwrap(), cx)
}

/// Type into the composer and send; the sim backend replies on timers, so
/// the test clock is advanced until the turn ends.
fn send_reply(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.backend = std::sync::Arc::new(SimBackend);
            ws.notify_on_done = true;
            ws.composer.update(cx, |composer, cx| {
                composer.set_value("hi", window, cx);
            });
            ws.send(window, cx);
        });
    });
    for _ in 0..32 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if !workspace.read_with(cx, |ws, _| ws.chats[0].running) {
            return;
        }
    }
    panic!("simulated reply never finished");
}

#[gpui_kit::test]
fn notifies_when_reply_finishes_unfocused(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    // Test windows open inactive; deactivate explicitly so the assertion
    // doesn't depend on platform defaults.
    cx.deactivate_window();
    send_reply(&workspace, cx);
    let notes = cx.delivered_system_notifications();
    assert_eq!(notes.len(), 1, "expected one reply-complete notification, got {notes:?}");
    assert_eq!(notes[0].title, "Rixl Code");
    assert!(notes[0].body.contains("reply complete"));
}

#[gpui_kit::test]
fn silent_when_reply_finishes_focused(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    send_reply(&workspace, cx);
    assert!(
        cx.delivered_system_notifications().is_empty(),
        "focused window must not post a system notification"
    );
}
