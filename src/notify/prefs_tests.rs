//! Headless UI tests for the notification-granularity controls: the
//! "Turn completion notifications" pick (Never / Only when unfocused /
//! Always) and the "Permission notifications" switch. Declared from
//! `notify.rs` — `main.rs` is at the SLOC cap.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::notify::prefs::NotifyPrefs;
use crate::workspace::Workspace;

fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
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

impl AgentBackend for OkBackend {
    fn name(&self) -> &'static str {
        "ok"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta("done".into()));
        let _ = tx.send(AgentEvent::Done);
        drop(tx);
        ReplyStream {
            events: rx,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Type into the composer and send; the backend replies on timers, so the
/// test clock is advanced until the turn ends.
fn send_reply(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.backend = std::sync::Arc::new(OkBackend);
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

/// "Always" posts the system banner even while the finished chat is on
/// screen — the option `notify_background` alone could never express.
#[gpui_kit::test]
fn always_mode_posts_system_notification_while_focused(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.update(|_window, cx| {
        workspace.update(cx, |ws, _cx| ws.notify_prefs.always = true);
    });
    send_reply(&workspace, cx);
    let notes = cx.delivered_system_notifications();
    assert_eq!(notes.len(), 1, "Always should post even while the finished chat is watched, got {notes:?}");
    assert_eq!(notes[0].title, "hi");
    assert_eq!(toast_count(cx), 1, "the in-app toast still accompanies it");
}

/// The segmented row writes the (always, background) flag pair and
/// persists both stores — `notify.json` and `settings.json`.
#[gpui_kit::test]
fn timing_row_click_updates_flags_and_persists(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_settings(window, cx)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-general").visible());
        window.click(("notify-timing", 0usize), cx); // Never
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!workspace.read(cx).notify_background, "Never clears the unwatched leg");
        assert!(!workspace.read(cx).notify_prefs.always);
        assert!(!crate::persist::load_settings().notify_background, "background flag should persist");
        assert!(!NotifyPrefs::load().always, "the sidecar should persist");

        window.click(("notify-timing", 2usize), cx); // Always
    });
    cx.update(|_window, cx| {
        assert!(workspace.read(cx).notify_prefs.always, "Always sets the always leg");
        assert!(workspace.read(cx).notify_background, "Always implies unfocused delivery");
        assert!(NotifyPrefs::load().always, "the sidecar should persist");
        assert!(crate::persist::load_settings().notify_background, "background flag should persist");
    });
}

/// "Permission notifications" is its own gate: approval toasts still fire
/// with turn-completion notices off — the decoupling the real app's
/// separate toggles offer.
#[gpui_kit::test]
fn approval_notice_survives_completion_toggle_off(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    let chat_id = ws.update(cx, |this, _| {
        this.notify_on_done = false;
        this.chats[0].id
    });
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.notify_approval(chat_id, "Run command: rm -rf ./build", window, cx));
    });
    assert_eq!(toast_count(cx), 1, "approvals should notify on their own toggle");
}

/// The "Permission notifications" switch writes the workspace flag and
/// persists it to `notify.json`.
#[gpui_kit::test]
fn approvals_switch_persists(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_settings(window, cx)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-general").visible());
        let toggle = window.find("toggle-notify-approvals");
        assert_eq!(toggle.checked(), Some(true), "switch should mirror the default-on flag");
        window.click("toggle-notify-approvals", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!workspace.read(cx).notify_prefs.approvals, "switch click should clear the flag");
        assert!(!NotifyPrefs::load().approvals, "switch click should persist the sidecar");
        assert_eq!(window.find("toggle-notify-approvals").checked(), Some(false));
    });
}

/// Approvals share the delivery pick: "Always" surfaces the system banner
/// while the waiting chat is on screen too.
#[gpui_kit::test]
fn approval_notice_posts_system_in_always_mode(cx: &mut TestAppContext) {
    let (ws, cx) = open_workspace(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let chat_id = ws.update(cx, |this, _| {
        this.notify_prefs.always = true;
        this.chats[0].id
    });
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.notify_approval(chat_id, "Run command: rm -rf ./build", window, cx));
    });
    let notes = cx.delivered_system_notifications();
    assert_eq!(notes.len(), 1, "Always should post the approval banner even while watched, got {notes:?}");
    assert!(notes[0].body.contains("Needs approval"), "body should name the ask, got {:?}", notes[0].body);
}
