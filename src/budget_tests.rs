//! Budget alerts: the per-chat/global spend cap checked at each turn's
//! end — banner + note on crossing, dismiss, re-arm on a raised cap,
//! per-chat override precedence, ephemeral chats, and the two edit
//! surfaces (⋯ menu dialog, General settings field). Same mount harness
//! as `rate_limit_tests.rs` — duplicated because sibling test files can't
//! share private helpers.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-budget-test-{}", std::process::id()));
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

impl ScriptedBackend {
    fn of(scripts: Vec<Vec<AgentEvent>>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { scripts: parking_lot::Mutex::new(scripts.into()) })
    }
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

/// One turn reporting `input` tokens — gpt-5 prices 1M input at $1.25.
fn turn(input: u64) -> Vec<AgentEvent> {
    vec![AgentEvent::TextDelta("ok".into()), AgentEvent::Usage { input, output: 0 }, AgentEvent::Done]
}

/// Price the active chat on gpt-5 and fold `input` tokens into its usage.
fn seed_spend(ws: &Entity<Workspace>, input: u64, cx: &mut VisualTestContext) {
    ws.update(cx, |this, _| {
        this.chats[0].model = "gpt-5".into();
        this.chats[0].usage.record(crate::usage::UsageReport::tokens(input, 0));
    });
}

/// The last assistant note's text, when the transcript's tail is a note.
fn last_note(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
    ws.read_with(cx, |ws, _| {
        ws.chats[0].messages.iter().rev().find_map(|m| match &m.kind {
            crate::model::MessageKind::Text(t) if m.role == crate::model::Role::Assistant => Some(t.to_string()),
            _ => None,
        })
    })
}

#[test]
fn crossing_the_cap_raises_banner_and_note() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let backend = ScriptedBackend::of(vec![turn(1_000_000)]);
    ws.update(cx, |this, cx| {
        this.chats[0].model = "gpt-5".into();
        this.set_budget_alert_usd(Some(1.0), cx);
    });
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| !ws.chats[0].running);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let banner = window.find("budget-banner");
        assert!(banner.visible(), "spend over the cap raises the banner");
        assert_eq!(banner.label(), Some("This chat has spent ~$1.25 (cap $1)"));
    });
    assert_eq!(last_note(&ws, cx).as_deref(), Some("**Budget alert:** this chat has spent ~$1.25 (cap $1)."));
}

#[test]
fn below_the_cap_stays_quiet() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let backend = ScriptedBackend::of(vec![turn(1_000_000)]);
    ws.update(cx, |this, cx| {
        this.chats[0].model = "gpt-5".into();
        this.set_budget_alert_usd(Some(5.0), cx);
    });
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| !ws.chats[0].running);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("budget-banner").is_none(), "spend under the cap stays quiet");
    });
    assert_eq!(last_note(&ws, cx).as_deref(), Some("ok"), "no budget note lands");
    assert_eq!(ws.read_with(cx, |ws, _| ws.chats[0].budget_alerted), None);
}

#[test]
fn chat_override_beats_the_global_cap() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_spend(&ws, 1_000_000, cx); // $1.25 spent
    ws.update(cx, |this, cx| {
        this.set_budget_alert_usd(Some(1.0), cx);
        this.set_chat_budget(this.chats[0].id, Some(5.0), cx);
    });
    let chat_id = ws.read_with(cx, |ws, _| ws.chats[0].id);
    ws.update(cx, |this, cx| this.finish_reply(chat_id, cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("budget-banner").is_none(), "the chat's higher cap wins over the global");
    });
    // Clearing the override drops back to the global cap — already crossed.
    ws.update(cx, |this, cx| this.set_chat_budget(chat_id, None, cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("budget-banner").visible(), "clearing the override re-checks the global cap");
    });
}

#[test]
fn dismiss_hides_the_banner() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_spend(&ws, 1_000_000, cx);
    ws.update(cx, |this, cx| this.set_budget_alert_usd(Some(1.0), cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("budget-banner").visible());
        window.click("dismiss-budget", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("budget-banner").is_none(), "dismiss hides the banner");
    });
    // A later turn under the same cap stays dismissed — no re-alert.
    let chat_id = ws.read_with(cx, |ws, _| ws.chats[0].id);
    ws.update(cx, |this, cx| this.finish_reply(chat_id, cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("budget-banner").is_none(), "the same cap stays dismissed");
    });
}

#[test]
fn raising_the_cap_rearms_the_alert() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let backend = ScriptedBackend::of(vec![turn(1_000_000), turn(1_000_000)]);
    ws.update(cx, |this, cx| {
        this.chats[0].model = "gpt-5".into();
        this.chats[0].title_generated = true; // keep the title turn from eating a script
        this.set_budget_alert_usd(Some(1.0), cx);
    });
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| !ws.chats[0].running);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("dismiss-budget", cx);
    });
    // Raise the cap above the current $1.25 spend — the alert re-arms.
    ws.update(cx, |this, cx| this.set_budget_alert_usd(Some(2.0), cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("budget-banner").is_none(), "the raised cap hides the stale alert");
    });
    // The next turn pushes spend to $2.50 — over the new cap, it fires again.
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| ws.chats[0].messages.len() >= 4 && !ws.chats[0].running);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let banner = window.find("budget-banner");
        assert!(banner.visible(), "crossing the raised cap alerts again");
        assert_eq!(banner.label(), Some("This chat has spent ~$2.5 (cap $2)"));
    });
}

#[test]
fn ephemeral_chat_alerts_without_disk() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let backend = ScriptedBackend::of(vec![turn(1_000_000)]);
    ws.update(cx, |this, cx| {
        this.new_temp_chat(cx);
        this.chats[this.active].model = "gpt-5".into();
        this.chats[this.active].title_generated = true;
        this.set_chat_budget(this.chats[this.active].id, Some(1.0), cx);
    });
    // The new-chat focus task clears the composer — let it land before
    // `send_with` types, or the send sees an empty draft.
    cx.run_until_parked();
    cx.update(|window, cx| send_with(&ws, &backend, window, cx));
    pump_until(cx, &ws, |ws| {
        ws.chats[ws.active].messages.iter().any(|m| m.role == crate::model::Role::Assistant) && !ws.chats[ws.active].running
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("budget-banner").visible(), "ephemeral chats alert in memory");
    });
    let dir = ws.read_with(cx, |ws, _| ws.project.chats_dir());
    let files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.flatten().collect()).unwrap_or_default();
    // The ephemeral chat sits in slot 1 — only the real chat's 0.json may exist.
    assert!(files.iter().all(|f| f.file_name() == "0.json"), "the temporary chat never reaches disk: {files:?}");
}
