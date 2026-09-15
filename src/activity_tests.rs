//! Tests for the activity center: feed recording (turn finished, approval,
//! error), the bell's unread badge, panel open/clear, entry navigation, and
//! persistence. Headless tests mount a real `Workspace`; the feed model and
//! its file round-trip are plain unit tests.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::activity::{ACTIVITY_LIMIT, ActivityEntry, ActivityFeed, ActivityKind};
use crate::backend::{AgentBackend, AgentEvent, ApprovalKind, ReplyStream};
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

/// Mount a `Workspace` in a headless window (same pattern as ui_tests).
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

/// A backend whose turn completes immediately — deterministic, unlike
/// `SimBackend`, which fails a quarter of replies at random.
struct OkBackend;

impl AgentBackend for OkBackend {
    fn name(&self) -> &'static str {
        "ok"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        stream(vec![AgentEvent::TextDelta("done".into()), AgentEvent::Done])
    }
}

/// A backend whose turn fails immediately — the channel closes after the
/// error, so the pump sees Disconnected and finishes the reply.
struct FailBackend;

impl AgentBackend for FailBackend {
    fn name(&self) -> &'static str {
        "fail"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        stream(vec![AgentEvent::Error("codex exited 1".into())])
    }
}

/// A backend that asks for approval, then hangs — the request's sender stays
/// alive so the pump never sees the channel close and the turn stays running.
struct AskBackend;

impl AgentBackend for AskBackend {
    fn name(&self) -> &'static str {
        "ask"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let (respond, _decisions) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::ApprovalRequest {
            ix: 0,
            kind: ApprovalKind::Command,
            detail: "rm -rf /tmp/x".into(),
            respond,
        });
        // Filler text pushes the card far above the viewport so the
        // click-through has something to scroll to.
        for _ in 0..40 {
            let _ = tx.send(AgentEvent::TextStart);
            let _ = tx.send(AgentEvent::TextDelta("filler line that pads the transcript".into()));
        }
        // Leak the sender: the turn must stay open so the approval card
        // remains pending for the click-through assertion.
        std::mem::forget(tx);
        ReplyStream {
            events: rx,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Type into the composer and send; the backend replies on timers, so the
/// test clock is advanced until the turn ends (or `wait_idle` is false and
/// the first events have landed).
fn send_reply(workspace: &Entity<Workspace>, backend: std::sync::Arc<dyn AgentBackend>, wait_idle: bool, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.backend = backend;
            ws.composer.update(cx, |composer, cx| {
                composer.set_value("hi", window, cx);
            });
            ws.send(window, cx);
        });
    });
    for _ in 0..32 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if !wait_idle || !workspace.read_with(cx, |ws, _| ws.chats[0].running) {
            return;
        }
    }
    panic!("simulated reply never finished");
}

#[test]
fn records_turn_finished() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.activity.entries.len(), 1, "a finished turn should record one entry");
        let e = &ws.activity.entries[0];
        assert_eq!(e.kind, ActivityKind::TurnFinished);
        assert_eq!(e.chat_title, "hi", "the entry carries the chat's title");
        assert!(e.body.contains("done"), "the entry previews the reply, got {:?}", e.body);
        assert!(e.unread, "fresh entries start unread");
        assert_eq!(ws.activity.unread_count(), 1);
    });
}

#[test]
fn records_error() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(FailBackend), true, cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.activity.entries.len(), 1);
        let e = &ws.activity.entries[0];
        assert_eq!(e.kind, ActivityKind::Error);
        assert!(e.body.contains("codex exited 1"), "the entry carries the failure, got {:?}", e.body);
    });
}

#[test]
fn records_approval() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(AskBackend), false, cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.activity.entries.len(), 1);
        let e = &ws.activity.entries[0];
        assert_eq!(e.kind, ActivityKind::Approval);
        assert!(e.body.contains("Run command"), "the entry names the approval kind, got {:?}", e.body);
        assert!(e.body.contains("rm -rf /tmp/x"), "the entry carries the detail, got {:?}", e.body);
    });
}

#[test]
fn badge_counts_and_clears_on_open() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("activity-bell").visible(), "the bell renders in the top bar");
        assert!(window.find("activity-badge").visible(), "one unread entry shows the badge");
        assert!(window.try_find("activity-panel").is_none(), "panel starts closed");

        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("activity-panel").visible(), "clicking the bell opens the panel");
        assert!(window.find(("activity-entry", 0usize)).visible(), "the entry row renders");
        assert!(window.try_find("activity-badge").is_none(), "opening the panel clears the badge");
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.activity_open);
        assert_eq!(ws.activity.unread_count(), 0, "opening the panel marks the feed read");
    });
}

#[test]
fn entry_click_opens_its_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    // A second chat takes focus; the entry must lead back to chat 0.
    cx.update(|_window, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    assert_eq!(ws.read_with(cx, |ws, _| ws.active), 1);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        window.click(("activity-entry", 0usize), cx);
        window.draw(cx).clear(cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, 0, "clicking the entry selects its chat");
        assert!(!ws.activity_open, "the panel closes after the click");
    });
}

#[test]
fn approval_entry_scrolls_to_pending_card() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(AskBackend), false, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The card sits at message 1 under 40 filler messages — the virtual
        // scroller is tail-anchored, so it isn't rendered yet.
        assert!(window.try_find(("approval", 1usize)).is_none(), "pending card starts off-screen");
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        window.click(("activity-entry", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("approval", 1usize)).visible(), "clicking the entry scrolls the pending card into view");
    });
}

#[test]
fn clear_empties_feed_and_file() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    let dir = ws.read_with(cx, |ws, _| ws.project.dir().to_path_buf());
    assert!(dir.join("activity.json").exists(), "recording persists the feed");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        window.click("activity-clear", cx);
        window.draw(cx).clear(cx);
    });
    ws.read_with(cx, |ws, _| assert!(ws.activity.entries.is_empty(), "Clear empties the feed"));
    assert!(!dir.join("activity.json").exists(), "Clear removes the persisted feed");
}

#[test]
fn feed_persists_across_load() {
    sandbox_home();
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let chat = crate::model::Chat::new(1, "hello");
    let mut feed = ActivityFeed::default();
    feed.push(ActivityEntry::new(ActivityKind::TurnFinished, &chat, "preview".to_string()));
    feed.push(ActivityEntry::new(ActivityKind::Error, &chat, "boom".to_string()));
    feed.persist(&dir);

    let loaded = ActivityFeed::load(&dir);
    assert_eq!(loaded.entries.len(), 2, "the feed round-trips through disk");
    assert_eq!(loaded.entries[0].kind, ActivityKind::TurnFinished);
    assert_eq!(loaded.entries[1].kind, ActivityKind::Error);
    assert_eq!(loaded.entries[1].body, "boom");
    assert_eq!(loaded.unread_count(), 2, "unread state persists");
    assert_eq!(loaded.entries[0].chat_created, chat.created_at, "the chat link survives");
}

#[test]
fn feed_is_bounded() {
    let chat = crate::model::Chat::new(1, "chat");
    let mut feed = ActivityFeed::default();
    for i in 0..ACTIVITY_LIMIT + 10 {
        feed.push(ActivityEntry::new(ActivityKind::TurnFinished, &chat, format!("turn {i}")));
    }
    assert_eq!(feed.entries.len(), ACTIVITY_LIMIT, "the feed caps at the limit");
    assert_eq!(feed.entries[0].body, "turn 10", "the oldest entries drop off first");
    assert_eq!(feed.entries[ACTIVITY_LIMIT - 1].body, format!("turn {}", ACTIVITY_LIMIT + 9));
}

#[test]
fn load_rejects_unknown_version() {
    sandbox_home();
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    std::fs::write(dir.join("activity.json"), r#"{"v":99,"entries":[]}"#).unwrap();
    assert!(ActivityFeed::load(&dir).entries.is_empty(), "an unknown version loads empty");
}
