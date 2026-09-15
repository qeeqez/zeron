//! Headless tests for mid-turn steer: the composer's Steer control injects
//! into a running turn when the backend supports it, and queues otherwise.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnHandle};
use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
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
    for _ in 0..200 {
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| !ws.project_files.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |composer, cx| composer.focus(window, cx));
        });
    });
    (workspace, cx)
}

/// Send `text` through the real input path: type, then Enter.
fn type_and_send(cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        window.input(text, cx);
        window.press("enter", cx);
    });
}

/// Advance the test clock until `cond` holds or the budget runs out.
fn until(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| cond(ws)) {
            return;
        }
    }
    panic!("condition never held");
}

/// Count user messages whose text contains `needle`.
fn user_msgs(ws: &Workspace, needle: &str) -> usize {
    ws.chats[ws.active]
        .messages
        .iter()
        .filter(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains(needle)))
        .count()
}

/// A turn handle that records steered text — stands in for the codex
/// app-server's stdin without spawning a process.
struct FakeSteer(std::sync::Arc<parking_lot::Mutex<Vec<String>>>);

impl TurnHandle for FakeSteer {
    fn kill(&self) {}

    fn steer(&self, text: &str) -> bool {
        self.0.lock().push(text.to_string());
        true
    }
}

/// A steerable backend whose turn never ends on its own — the sender is
/// held so the reply stays open until the test drops the workspace.
struct SteerBackend {
    steered: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for SteerBackend {
    fn name(&self) -> &'static str {
        "steer-stub"
    }

    fn models(&self) -> Vec<crate::model::ModelInfo> {
        vec![crate::model::ModelInfo { id: "m".into(), label: "M".into(), description: "".into() }]
    }

    fn supports_steer(&self) -> bool {
        true
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta("working".into()));
        std::mem::forget(tx);
        ReplyStream {
            events,
            child: Some(std::sync::Arc::new(FakeSteer(self.steered.clone()))),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// While a steerable turn runs, the composer offers Steer + Queue next to
/// Stop; clicking Steer injects the text into the turn and appends it to
/// the transcript as a user message.
#[gpui_kit::test]
fn steer_button_injects_into_running_turn(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    let steered = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(SteerBackend { steered: steered.clone() });
        });
    });

    type_and_send(cx, "first");
    until(&workspace, cx, |ws| ws.chats[ws.active].running);

    // The running turn exposes Steer and Queue next to Stop.
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("steer").visible(), "steer control should render while a steerable turn runs");
        assert!(window.find("queue").visible());
    });

    // Type the steer text, then click Steer — it lands on the turn's
    // handle and in the transcript, and the composer clears.
    cx.update(|window, cx| {
        window.input("also do this", cx);
        window.click("steer", cx);
    });
    assert_eq!(steered.lock().as_slice(), ["also do this"]);
    let ws = workspace.read_with(cx, |ws, app| {
        (user_msgs(ws, "also do this"), ws.composer.read(app).value().to_string(), ws.send_queue.queued(ws.chats[ws.active].id).len())
    });
    assert_eq!(ws.0, 1, "steered text should appear as a user message");
    assert!(ws.1.is_empty(), "composer should clear after a steer");
    assert_eq!(ws.2, 0, "a successful steer never touches the queue");
}

/// Enter still queues while a steerable turn runs — Steer is the explicit
/// opt-in, Queue the default.
#[gpui_kit::test]
fn enter_still_queues_while_steerable(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    let steered = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(SteerBackend { steered: steered.clone() });
        });
    });
    type_and_send(cx, "first");
    until(&workspace, cx, |ws| ws.chats[ws.active].running);
    type_and_send(cx, "queued-not-steered");
    let chat_id = workspace.read_with(cx, |ws, _| ws.chats[ws.active].id);
    assert_eq!(workspace.read_with(cx, |ws, _| ws.send_queue.queued(chat_id).len()), 1);
    assert!(steered.lock().is_empty(), "Enter must not steer");
}

/// A backend without mid-turn input gets no Steer control, and calling
/// `send_steer` anyway falls back to the queue.
#[gpui_kit::test]
fn unsupported_backend_queues_instead(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(crate::backend::SimBackend);
        });
    });
    type_and_send(cx, "first");
    until(&workspace, cx, |ws| ws.chats[ws.active].running);

    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("steer").is_none(), "sim turns can't steer — no Steer control");
    });

    // send_steer on an unsteerable turn queues the message.
    cx.update(|window, cx| {
        window.input("queued instead", cx);
        workspace.update(cx, |ws, cx| ws.send_steer(window, cx));
    });
    let chat_id = workspace.read_with(cx, |ws, _| ws.chats[ws.active].id);
    let queued = workspace.read_with(cx, |ws, _| ws.send_queue.queued(chat_id));
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].text, "queued instead");
}
