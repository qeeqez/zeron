//! Headless tests for the usage meter and popover: `AgentEvent::Usage`
//! streams fold onto the chat, the composer indicator updates live, and
//! clicking the meter opens the per-turn/cost breakdown. The plain unit
//! test at the bottom lives here because `usage.rs` is at the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::usage::{ChatUsage, UsageReport};
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

/// Drive the fake executor until `done` observes the applied state. The
/// backend pump runs on a real `std::thread` while the reply task polls
/// its channel on a fake-clock timer, so `advance_clock`/`run_until_parked`
/// alone can't guarantee delivery — under parallel test load the OS may
/// not schedule the pump within a fixed iteration count. Poll on a
/// real-time deadline instead, sleeping so the pump gets a core.
fn pump_until(cx: &mut VisualTestContext, ws: &Entity<Workspace>, done: impl Fn(&Workspace) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| done(ws)) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "pump thread never delivered the usage events");
        std::thread::sleep(std::time::Duration::from_millis(1));
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
    pump_until(cx, &ws, |ws| ws.chats[0].usage.total == 450);
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
    pump_until(cx, &ws, |ws| ws.chats[0].usage.context_used == Some(1_200));
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

#[test]
fn titlebar_meter_shows_fill_or_token_fallback() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("context-meter").is_none(), "no reports yet — chip stays hidden");
    });
    // Token backend: no window size → the chip shows cumulative tokens.
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(100, 40)]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let chip = window.find("context-meter");
        assert!(chip.visible());
        // gpt-5 is priced: 100 in + 40 out ≈ $0.000525 — the aria label
        // carries the exact counts plus the estimate and its rate.
        assert_eq!(chip.label(), Some("140 tokens this chat · ~$0.000525 est · $1.25 in / $10 out per Mtok"));
    });
    // Occupancy backend: the chip switches to the window's fill percent.
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::occupancy(170_000, 200_000)]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find("context-meter").label(), Some("170,000 / 200,000 tokens · ~$0.000525 est · $1.25 in / $10 out per Mtok"));
    });
}

#[test]
fn chip_shows_no_cost_for_local_models() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // An ollama-style model id has no pricing row — the chip shows tokens
    // only, no cost suffix.
    seed_usage(&ws, cx, 0, "llama3.2", &[UsageReport::tokens(100, 40)]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find("context-meter").label(), Some("140 tokens this chat"));
    });
}

#[test]
fn cost_accumulates_across_turns() {
    let mut u = ChatUsage::default();
    u.record(UsageReport::tokens(1_000_000, 0));
    u.begin_turn();
    u.record(UsageReport::tokens(0, 500_000));
    // gpt-5: $1.25/1M in + $10/1M out → the running estimate prices
    // both turns, not just the in-flight one.
    assert_eq!(u.cost("gpt-5"), Some(6.25));
}

/// Seed a chat's usage without driving a backend — the popover reads the
/// folded state, not the stream.
fn seed_usage(ws: &Entity<Workspace>, cx: &mut VisualTestContext, chat: usize, model: &str, reports: &[UsageReport]) {
    ws.update(cx, |this, _| {
        this.chats[chat].model = model.into();
        for &r in reports {
            this.chats[chat].usage.record(r);
        }
    });
}

#[test]
fn meter_click_opens_the_usage_breakdown() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Two turns: one completed (folded by begin_turn), one in flight.
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(100, 40)]);
    ws.update(cx, |this, _| this.chats[0].usage.begin_turn());
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(50, 10)]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("usage-meter", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("usage-breakdown").visible(), "meter click opens the breakdown");
        assert_eq!(window.find("usage-total").label(), Some("This chat: 200 tok"));
        assert_eq!(window.find(("usage-turn", 0usize)).label(), Some("Turn 1: 100 in · 40 out"));
        assert_eq!(window.find(("usage-turn", 1usize)).label(), Some("Turn 2: 50 in · 10 out"));
        // gpt-5 is priced: 140·$1.25/1M + 60·$10/1M ≈ $0.000775.
        let cost = window.find("usage-cost").label().unwrap_or_default().to_string();
        assert!(cost.starts_with("Est. cost: ~$"), "priced model shows an estimate, got {cost}");
        assert!(window.try_find("usage-session").is_none(), "one chat — no session row");
    });
}

#[test]
fn unknown_model_shows_tokens_only() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_usage(&ws, cx, 0, "sim-x", &[UsageReport::tokens(100, 40)]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("usage-meter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("usage-cost").label(), Some("Est. cost: —"), "unpriced model shows no estimate");
        assert_eq!(window.find("usage-total").label(), Some("This chat: 140 tok"));
    });
}

#[test]
fn session_row_aggregates_across_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(100, 40)]);
    ws.update(cx, |this, cx| {
        this.new_chat(cx);
        this.chats[this.active].model = "sim-x".into();
        this.chats[this.active].usage.record(UsageReport::tokens(10, 10));
    });
    let s = ws.read_with(cx, |ws, _| ws.session_usage());
    assert_eq!(s.total, 160);
    assert!(s.cost_partial, "sim-x has no pricing — the sum is a lower bound");
    assert!(s.cost > 0., "the priced chat still contributes");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("usage-meter", cx);
        window.draw(cx).clear(cx);
        let session = window.find("usage-session").label().unwrap_or_default().to_string();
        assert!(session.starts_with("Session: 160 tok"), "session row totals both chats, got {session}");
    });
}
