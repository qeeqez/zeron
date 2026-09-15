//! Headless tests for the usage meter: `AgentEvent::Usage` streams fold
//! onto the chat and the composer indicator updates live.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-usage-test-{}", std::process::id()));
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

/// A backend that streams canned usage reports, then hangs — the turn
/// stays running so the test observes the meter mid-stream.
struct UsageBackend {
    name: &'static str,
    events: Vec<AgentEvent>,
}

impl AgentBackend for UsageBackend {
    fn name(&self) -> &'static str {
        self.name
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        for e in &self.events {
            let _ = tx.send(e.clone());
        }
        std::mem::forget(tx); // producer never exits — the turn stays live
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

fn send_with(ws: &Entity<Workspace>, backend: UsageBackend, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.backend = std::sync::Arc::new(backend);
        this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
        this.send(window, cx);
    });
}

fn pump(cx: &mut VisualTestContext) {
    for _ in 0..8 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
    }
}

#[test]
fn usage_events_accumulate_and_update_the_meter() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("usage-meter").is_none(), "no reports yet — meter stays hidden");
        send_with(
            &ws,
            UsageBackend {
                name: "stub",
                // Reports are the turn's running total, not deltas.
                events: vec![AgentEvent::Usage { input: 100, output: 40 }, AgentEvent::Usage { input: 300, output: 150 }],
            },
            window,
            cx,
        );
    });
    pump(cx);
    ws.read_with(cx, |ws, _| {
        let usage = &ws.chats[0].usage;
        assert_eq!(usage.turn, 450, "turn sums the report deltas");
        assert_eq!(usage.total, 450);
        assert_eq!(usage.context, None, "token backend reports no window");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let meter = window.find("usage-meter");
        assert!(meter.visible());
        assert_eq!(meter.label(), Some("+450 · 450 tok"), "no context size → token count only");
    });
}

#[test]
fn acp_usage_reports_fill_the_context_meter() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        // acp's usage_update carries context occupancy: used of size.
        send_with(
            &ws,
            UsageBackend {
                name: "acp",
                events: vec![AgentEvent::Usage { input: 1_200, output: 200_000 }],
            },
            window,
            cx,
        );
    });
    pump(cx);
    ws.read_with(cx, |ws, _| {
        let usage = &ws.chats[0].usage;
        assert_eq!(usage.turn, 0, "occupancy reports carry no turn tokens");
        assert_eq!(usage.total, 0);
        assert_eq!(usage.context, Some(200_000));
        assert_eq!(usage.context_used, Some(1_200));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find("usage-meter").label(), Some("1.2k / 200k"));
    });
}
