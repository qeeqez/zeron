//! Headless UI tests for reply-completion notifications: drive a real
//! `Workspace` window through send → reply done and assert the platform
//! notification surface. Requires gpui-kit's `test-support` feature.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, SystemNotificationResponse, TestAppContext, VisualTestContext, px, size};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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

/// A backend whose turn completes immediately — deterministic, unlike
/// `SimBackend`, which fails a quarter of replies at random.
struct OkBackend;

/// A backend whose turn fails immediately — the channel closes after the
/// error, so the pump sees Disconnected and finishes the reply.
struct FailBackend;

fn stream(events: Vec<AgentEvent>) -> ReplyStream {
    let (tx, rx) = std::sync::mpsc::channel();
    for e in events {
        let _ = tx.send(e);
    }
    drop(tx);
    ReplyStream {
        events: rx,
        child: None,
        cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

impl AgentBackend for OkBackend {
    fn name(&self) -> &'static str {
        "ok"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        stream(vec![AgentEvent::TextDelta("done".into()), AgentEvent::Done])
    }
}

impl AgentBackend for FailBackend {
    fn name(&self) -> &'static str {
        "fail"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        stream(vec![AgentEvent::Error("codex exited 1".into())])
    }
}

/// Type into the composer and send; the backend replies on timers, so the
/// test clock is advanced until the turn ends.
fn send_reply(workspace: &Entity<Workspace>, backend: std::sync::Arc<dyn AgentBackend>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.backend = backend;
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

/// In-app toasts mounted under the Root's notification layer.
fn toast_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|_window, cx| {
        let Some(Some(root)) = _window.root::<Root>() else { return 0 };
        root.read(cx).notification.read(cx).notifications().len()
    })
}

#[gpui_kit::test]
fn notifies_when_reply_finishes_unfocused(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    // Test windows open inactive; deactivate explicitly so the assertion
    // doesn't depend on platform defaults.
    cx.deactivate_window();
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    let notes = cx.delivered_system_notifications();
    assert_eq!(notes.len(), 1, "expected one reply-complete notification, got {notes:?}");
    assert_eq!(notes[0].title, "hi", "system notification headline is the chat title");
    assert!(notes[0].body.contains("done"), "body should preview the reply, got {:?}", notes[0].body);
    assert_eq!(toast_count(cx), 1, "an in-app toast should accompany the system notification");
}

#[gpui_kit::test]
fn silent_when_reply_finishes_focused(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    assert!(cx.delivered_system_notifications().is_empty(), "focused window must not post a system notification");
    assert_eq!(toast_count(cx), 1, "the in-app toast still shows while focused");
}

#[gpui_kit::test]
fn failed_reply_notifies_error_not_success(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.deactivate_window();
    send_reply(&workspace, std::sync::Arc::new(FailBackend), cx);
    let notes = cx.delivered_system_notifications();
    assert_eq!(notes.len(), 1, "expected one failure notification, got {notes:?}");
    assert_eq!(notes[0].title, "hi");
    assert!(
        notes[0].body.contains("Reply failed") && notes[0].body.contains("codex exited 1"),
        "failure body should carry the backend error, got {:?}",
        notes[0].body
    );
    assert!(!notes[0].body.contains("Reply complete"), "a failed turn must not report success");
    assert_eq!(toast_count(cx), 1, "an in-app error toast should accompany the system notification");
}

#[gpui_kit::test]
fn clicking_system_notification_opens_the_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.deactivate_window();
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    // A second chat takes focus; the notification must lead back to chat 0.
    cx.update(|_window, cx| workspace.update(cx, |ws, cx| ws.new_chat(cx)));
    assert_eq!(workspace.read_with(cx, |ws, _| ws.active), 1);

    let tag = cx.delivered_system_notifications()[0].tag.clone();
    cx.simulate_system_notification_response(SystemNotificationResponse { tag, action_id: None });
    cx.run_until_parked();

    assert_eq!(workspace.read_with(cx, |ws, _| ws.active), 0, "clicking the notification should select its chat");
    cx.update(|window, _| assert!(window.is_window_active(), "clicking should activate the window"));
    assert!(cx.delivered_system_notifications().is_empty(), "the clicked notification is retracted");
}

#[gpui_kit::test]
fn notification_click_closes_settings_overlay(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.deactivate_window();
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    // Settings open over the chat; the notification click must dismiss it,
    // not leave the overlay covering the chat it just switched to.
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_settings(window, cx)));
    assert!(workspace.read_with(cx, |ws, _| ws.settings_open));

    let tag = cx.delivered_system_notifications()[0].tag.clone();
    cx.simulate_system_notification_response(SystemNotificationResponse { tag, action_id: None });
    cx.run_until_parked();

    assert!(!workspace.read_with(cx, |ws, _| ws.settings_open), "notification click must close the settings overlay");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("settings-screen").is_none(), "settings overlay must not still cover the chat");
    });
}

#[gpui_kit::test]
fn clicking_toast_opens_the_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    cx.update(|_window, cx| workspace.update(cx, |ws, cx| ws.new_chat(cx)));
    assert_eq!(workspace.read_with(cx, |ws, _| ws.active), 1);

    // The enter animation slides the toast in from above the window; its
    // center can still be off-screen, so click near the bottom edge —
    // inside the window even mid-animation.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let b = window.find("notification").bounds();
        let y_inside = (b.size.height - px(5.)).max(px(0.));
        window.click_at("notification", gpui_kit::point(px(20.), y_inside), cx);
    });
    cx.run_until_parked();
    assert_eq!(workspace.read_with(cx, |ws, _| ws.active), 0, "clicking the toast should select its chat");
}

#[gpui_kit::test]
fn plays_sound_when_reply_finishes(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    // `notify_sound` defaults on; the test platform's bell is silent, so the
    // observable signal is the SOUND_PLAYS counter.
    let before = crate::notify::SOUND_PLAYS.load(std::sync::atomic::Ordering::Relaxed);
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    let plays = crate::notify::SOUND_PLAYS.load(std::sync::atomic::Ordering::Relaxed) - before;
    assert_eq!(plays, 1, "a finished turn should chime once");
}

#[gpui_kit::test]
fn no_sound_when_disabled(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|_window, cx| {
        workspace.update(cx, |ws, _cx| ws.notify_sound = false);
    });
    let before = crate::notify::SOUND_PLAYS.load(std::sync::atomic::Ordering::Relaxed);
    send_reply(&workspace, std::sync::Arc::new(OkBackend), cx);
    assert_eq!(crate::notify::SOUND_PLAYS.load(std::sync::atomic::Ordering::Relaxed), before, "notify_sound off must silence the chime");
}

#[gpui_kit::test]
fn sound_switch_persists(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_settings(window, cx)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-general").visible());
        let toggle = window.find("toggle-notify-sound");
        assert_eq!(toggle.checked(), Some(true), "switch should mirror the default-on flag");
        window.click("toggle-notify-sound", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!workspace.read(cx).notify_sound, "switch click should clear the flag");
        assert!(!crate::persist::load_settings().notify_sound, "switch click should persist");
        assert_eq!(window.find("toggle-notify-sound").checked(), Some(false));
    });
}
