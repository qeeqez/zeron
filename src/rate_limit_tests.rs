//! Rate-limit surfacing: signal parsing (codex snapshot, claude frame,
//! error text), the chat banner's set/clear lifecycle, and quota rows in
//! the usage popover.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::rate_limit::{RateLimit, RateWindow};
use crate::usage::ChatUsage;
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-ratelimit-test-{}", std::process::id()));
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

/// A backend that plays a scripted event list per turn — each `send`
/// shifts one script off the queue.
struct ScriptedBackend {
    scripts: parking_lot::Mutex<std::collections::VecDeque<Vec<AgentEvent>>>,
}

impl AgentBackend for ScriptedBackend {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        for e in self.scripts.lock().pop_front().unwrap_or_default() {
            let _ = tx.send(e);
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

fn send_with(ws: &Entity<Workspace>, backend: &std::sync::Arc<ScriptedBackend>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.backend = backend.clone();
        this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
        this.send(window, cx);
    });
}

/// Drive the fake executor until `done` observes the applied state — the
/// pump runs on a real thread, so poll on a real-time deadline.
fn pump_until(cx: &mut VisualTestContext, ws: &Entity<Workspace>, done: impl Fn(&Workspace) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| done(ws)) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "pump thread never delivered the events");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn codex_snapshot_parses_windows_and_reached() {
    let rl = RateLimit::from_codex(&serde_json::json!({
        "rateLimits": {
            "limitId": "codex",
            "primary": {"usedPercent": 25, "windowDurationMins": 300, "resetsAt": 4_000_000_000u64},
            "secondary": {"usedPercent": 97, "windowDurationMins": 10080, "resetsAt": 4_000_100_000u64},
            "planType": "pro",
            "rateLimitReachedType": "rate_limit_reached"
        }
    }))
    .unwrap();
    assert!(rl.limited);
    assert_eq!(rl.primary.unwrap().used_percent, 25.);
    assert_eq!(rl.secondary.unwrap().window_mins, Some(10080));
    assert_eq!(rl.reset_time(), Some(4_000_000_000));
    // The exec JSONL spelling (snake_case, bare snapshot) parses the same.
    let rl = RateLimit::from_codex(&serde_json::json!({
        "primary": {"used_percent": 40, "window_minutes": 300, "resets_at": 4_000_000_000u64},
        "secondary": null,
        "rate_limit_reached_type": null
    }))
    .unwrap();
    assert!(!rl.limited);
    assert_eq!(rl.primary.unwrap().used_percent, 40.);
}

#[test]
fn claude_frame_and_error_text_set_the_limit() {
    let rl = RateLimit::from_claude(&serde_json::json!({
        "status": "rejected",
        "resetsAt": 4_000_000_000u64,
        "rateLimitType": "five_hour"
    }))
    .unwrap();
    assert!(rl.limited);
    assert_eq!(rl.primary.unwrap().window_mins, Some(300));
    // "allowed" still emits — it clears a prior limit.
    let rl = RateLimit::from_claude(&serde_json::json!({"status": "allowed"})).unwrap();
    assert!(!rl.limited);
    assert!(rl.primary.is_none(), "a fieldless window is noise");

    let rl = RateLimit::from_error("You've hit your usage limit. Upgrade to Pro, or try again at 3:04 PM.").unwrap();
    assert!(rl.limited);
    assert_eq!(rl.reset_hint.as_deref(), Some("3:04 PM"));
    let rl = RateLimit::from_error("Rate limit exceeded. Try again in 35 seconds.").unwrap();
    assert_eq!(rl.reset_hint.as_deref(), Some("in 35 seconds"));
    assert!(RateLimit::from_error("http 429: too many requests").is_some());
    assert!(RateLimit::from_error("error 4290 while compiling").is_none(), "429 must be a whole token");
    assert!(RateLimit::from_error("disk quota exceeded").is_none(), "disk quota isn't a rate limit");
    assert!(RateLimit::from_error("connection reset by peer").is_none());
}

#[test]
fn banner_text_and_clear_on_success() {
    let now = std::time::SystemTime::now();
    let soon = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
    let mut usage = ChatUsage::default();
    assert!(usage.rate_limit.is_none());

    // An error-derived limit merges onto a reported snapshot instead of
    // dropping its windows.
    usage.record_rate_limit(RateLimit {
        limited: false,
        reset_hint: None,
        primary: Some(RateWindow {
            used_percent: 25.,
            window_mins: Some(300),
            resets_at: Some(soon),
        }),
        secondary: None,
    });
    usage.record_rate_limit(RateLimit::from_error("rate limit exceeded: try again at 3:04 PM").unwrap());
    let rl = usage.rate_limit.as_ref().unwrap();
    assert!(rl.limited);
    assert_eq!(rl.primary.unwrap().used_percent, 25., "windows survive the merge");
    assert_eq!(rl.banner(now).as_deref(), Some("Rate limited — resets 3:04 PM"));

    // A clean turn lifts the banner but keeps the quota rows.
    usage.clear_limited();
    let rl = usage.rate_limit.as_ref().unwrap();
    assert!(!rl.limited);
    assert!(rl.banner(now).is_none(), "25% is below the warning threshold");
    assert!(rl.primary.is_some());

    // A near-full window warns without a hard limit.
    usage.record_rate_limit(RateLimit {
        limited: false,
        reset_hint: None,
        primary: Some(RateWindow {
            used_percent: 92.,
            window_mins: Some(300),
            resets_at: Some(soon),
        }),
        secondary: None,
    });
    assert_eq!(usage.rate_limit.as_ref().unwrap().banner(now).as_deref(), Some("Approaching rate limit — 92% used"));
}

#[test]
fn rate_limited_turn_shows_the_banner() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let backend = std::sync::Arc::new(ScriptedBackend {
        scripts: parking_lot::Mutex::new(
            [
                vec![
                    AgentEvent::RateLimit(RateLimit {
                        limited: true,
                        reset_hint: Some("3:04 PM".into()),
                        primary: Some(RateWindow {
                            used_percent: 40.,
                            window_mins: Some(300),
                            resets_at: Some(4_000_000_000),
                        }),
                        secondary: None,
                    }),
                    AgentEvent::Error("rate limit exceeded".into()),
                    AgentEvent::Done,
                ],
                vec![AgentEvent::TextDelta("ok".into()), AgentEvent::Done],
            ]
            .into(),
        ),
    });
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| !ws.chats[0].running);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let banner = window.find("rate-limit-banner");
        assert!(banner.visible(), "a throttled turn raises the banner");
        assert_eq!(banner.label(), Some("Rate limited — resets 3:04 PM"));
        // The banner's own Retry replaces the generic failure row.
        assert!(window.try_find("retry-failed").is_none());
        assert!(window.find("retry-rate-limit").visible());
    });
    // The next turn succeeds — the banner clears.
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| !ws.chats[0].running);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("rate-limit-banner").is_none(), "a successful turn clears the banner");
    });
    ws.read_with(cx, |ws, _| {
        let rl = ws.chats[0].usage.rate_limit.as_ref().unwrap();
        assert!(!rl.limited);
        assert!(rl.primary.is_some(), "quota windows survive the clear");
    });
}

#[test]
fn quota_windows_fold_into_the_popover() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let soon = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
    ws.update(cx, |this, _| {
        this.chats[0].model = "gpt-5".into();
        this.chats[0].usage.record(crate::usage::UsageReport::tokens(100, 40));
        this.chats[0].usage.record_rate_limit(RateLimit {
            limited: false,
            reset_hint: None,
            primary: Some(RateWindow {
                used_percent: 25.,
                window_mins: Some(300),
                resets_at: Some(soon),
            }),
            secondary: Some(RateWindow { used_percent: 80., window_mins: Some(10080), resets_at: None }),
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("usage-meter", cx);
        window.draw(cx).clear(cx);
        let quota = window.find(("usage-quota", 0usize)).label().unwrap_or_default().to_string();
        assert!(quota.starts_with("5h limit: 25% · resets "), "quota row carries the reset time, got {quota}");
        assert_eq!(window.find(("usage-quota", 1usize)).label(), Some("1w limit: 80%"));
        assert!(window.try_find("usage-limit").is_none(), "not limited — no reached row");
    });
}
