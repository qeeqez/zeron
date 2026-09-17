//! Headless tests for "stop all": every running chat's turn must signal its
//! backend's cancel flag, the sidebar bar and palette command appear only at
//! 2+ running chats, and a queued message on a stopped chat stays queued.
//! Mount pattern matches `backend_run_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, ReplyStream};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-stopall-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    sandbox_home();
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

/// A backend whose streams never produce an event — each `send` records its
/// own `cancelled` flag so the test can see which turns stop-all killed.
struct MultiHangBackend {
    cancelled: parking_lot::Mutex<Vec<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}

impl AgentBackend for MultiHangBackend {
    fn name(&self) -> &'static str {
        "multi-hang"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        std::mem::forget(tx); // producer never exits — the pump blocks on recv
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.cancelled.lock().push(cancelled.clone());
        ReplyStream { events, child: None, cancelled }
    }
}

/// Send "hi" on the active chat — starts a hanging turn.
fn send_here(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    cx.run_until_parked();
}

/// Open a fresh chat and start a hanging turn on it — the previous chat
/// keeps streaming in the background.
fn new_chat_and_send(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.new_chat(cx));
    });
    send_here(ws, cx);
}

#[test]
fn stop_all_halts_every_running_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let backend = std::sync::Arc::new(MultiHangBackend { cancelled: parking_lot::Mutex::new(Vec::new()) });
    cx.update(|_window, cx| {
        ws.update(cx, |this, _cx| this.backend = backend.clone());
    });

    send_here(&ws, cx);
    new_chat_and_send(&ws, cx);
    // A third chat stays idle — stop-all must not touch it.
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.new_chat(cx));
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.running_chats()), 2, "two chats should be streaming");

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.stop_all_replies(cx));
    });
    cx.run_until_parked();

    assert_eq!(ws.read_with(cx, |ws, _| ws.running_chats()), 0, "stop-all leaves nothing running");
    let flags = backend.cancelled.lock();
    assert_eq!(flags.len(), 2, "exactly two turns ran");
    assert!(flags.iter().all(|f| f.load(std::sync::atomic::Ordering::SeqCst)), "every running turn's stream must signal cancel");
}

/// A queued message on a stopped chat stays queued — stop-all reuses the
/// per-chat stop, which never drops the send queue.
#[test]
fn stop_all_keeps_queued_messages() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, _cx| {
            this.backend = std::sync::Arc::new(MultiHangBackend { cancelled: parking_lot::Mutex::new(Vec::new()) });
        });
    });

    // Chat A: run a turn, then queue a message behind it.
    let chat_a = cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |composer, cx| composer.set_value("first", window, cx));
            this.send(window, cx);
            this.composer.update(cx, |composer, cx| composer.set_value("queued-a", window, cx));
            this.send(window, cx);
            this.chats[this.active].id
        })
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.send_queue.queued(chat_a).len()), 1, "mid-reply send should queue");

    // Chat B runs concurrently; A's drain exits once A backgrounds.
    new_chat_and_send(&ws, cx);
    assert_eq!(ws.read_with(cx, |ws, _| ws.running_chats()), 2, "both chats should be streaming");

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.stop_all_replies(cx));
    });
    cx.run_until_parked();

    let queued = ws.read_with(cx, |ws, _| ws.send_queue.queued(chat_a));
    assert_eq!(queued.len(), 1, "the stopped chat's queue must survive");
    assert_eq!(queued[0].text, "queued-a");
}

/// The sidebar's stop-all bar renders only while 2+ chats stream, and
/// clicking it halts every running chat.
#[test]
fn sidebar_stop_all_bar_gates_and_stops() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("stop-all").is_none(), "no bar with nothing running");

        ws.update(cx, |this, _cx| this.chats[0].running = true);
        window.draw(cx).clear(cx);
        assert!(window.try_find("stop-all").is_none(), "one running chat keeps the bar hidden");

        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[this.active].running = true;
        });
        window.draw(cx).clear(cx);
        assert!(window.find("stop-all").visible(), "2+ running chats show the bar");

        window.click("stop-all", cx);
    });
    cx.run_until_parked();
    assert_eq!(ws.read_with(cx, |ws, _| ws.running_chats()), 0, "clicking the bar stops every chat");
}

/// The palette's "Stop All Replies" command exists only while 2+ chats run.
#[test]
fn palette_stop_all_is_gated() {
    let has_stop_all = |running: usize| {
        crate::palette_items::build_entries(&[], "stop all", running)
            .iter()
            .any(|e| matches!(e, crate::palette_items::Entry::Command(spec) if spec.label == "Stop All Replies"))
    };
    assert!(!has_stop_all(0), "hidden with nothing running");
    assert!(!has_stop_all(1), "hidden with a single running chat");
    assert!(has_stop_all(2), "visible once two chats run");
    // The empty-query listing gates the same way.
    let listed = crate::palette_items::build_entries(&[], "", 2)
        .iter()
        .any(|e| matches!(e, crate::palette_items::Entry::Command(spec) if spec.label == "Stop All Replies"));
    assert!(listed, "the command joins the empty-query list when gated in");
}

/// Confirming the palette's stop-all row runs the workspace method — the
/// `Run` effect resolves through `entry_at` like every other command.
#[test]
fn palette_stop_all_confirms() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.chats[0].running = true;
            this.new_chat(cx);
            this.chats[this.active].running = true;
        });
    });
    let chats = ws.read_with(cx, |ws, _| ws.palette_chats());
    let running = ws.read_with(cx, |ws, _| ws.running_chats());
    let entries = crate::palette_items::build_entries(&chats, "stop all", running);
    let row = entries
        .iter()
        .position(|e| matches!(e, crate::palette_items::Entry::Command(spec) if spec.label == "Stop All Replies"))
        .expect("stop-all command should be listed");
    let Some(crate::palette_items::Entry::Command(spec)) =
        crate::palette_items::entry_at(&chats, "stop all", gpui_kit::component::IndexPath::new(row).section(0), running)
    else {
        panic!("entry_at should resolve the stop-all row");
    };
    let crate::palette_items::Effect::Run(run) = spec.effect else {
        panic!("stop-all should run, not dispatch");
    };
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| run(this, window, cx));
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.running_chats()), 0, "confirming the row stops every chat");
}
