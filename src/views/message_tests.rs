//! Headless tests for the message row: hover-revealed actions (copy,
//! retry, view-raw, rating, speak), the duration label, and retry
//! re-sending the last user prompt.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{ChatMessage, MessageKind, PlanStatus, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-msg-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
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

/// Seed the active chat with a completed assistant turn.
fn seed_reply(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        this.push_note("reply body".into(), cx);
        this.chats[this.active].last_turn = Some(std::time::Duration::from_secs(7));
    });
}

/// Push a user message without starting a reply — retry tests seed the
/// prompt directly instead of going through `send`.
fn seed_user(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

#[test]
fn assistant_actions_reveal_on_hover() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_reply(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Ghost icons exist but stay invisible until the row is hovered.
        assert!(!window.find(("copy", 0usize)).visible(), "copy hidden before hover");
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        for id in ["copy", "raw", "retry", "up", "down", "speak"] {
            assert!(window.find((id, 0usize)).visible(), "{id} should reveal on hover");
        }
    });
}

#[test]
fn completed_turn_shows_duration_and_feedback_toggles() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_reply(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("worked", 0usize)).visible(), "duration label should show");
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        window.click(("up", 0usize), cx);
        assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(true));
        window.draw(cx).clear(cx);
        window.click(("down", 0usize), cx);
        assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(false));
    });
    // The seeded message is an assistant note — verify role for sanity.
    app.read(|cx| assert!(matches!(ws.read(cx).chats[0].messages[0].role, Role::Assistant)));
}
/// Records every prompt the workspace sends so retry can be asserted
/// against the real `run_backend` path. The stream ends immediately.
struct RecordingBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        self.sent.lock().push(prompt.to_string());
        let (_tx, events) = std::sync::mpsc::channel();
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// The hover toolbar's retry button drops the last assistant reply and
/// re-sends the last user prompt.
#[test]
fn retry_resends_last_user_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    ws.update(cx, |this, _cx| {
        this.backend = std::sync::Arc::new(RecordingBackend { sent: sent.clone() });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 1usize), cx);
        window.draw(cx).clear(cx);
        window.click(("retry", 1usize), cx);
    });
    let prompts = sent.lock().clone();
    assert_eq!(prompts, ["fix the bug"], "retry must re-send the last user prompt");
    app.read(|cx| {
        let msgs = &ws.read(cx).chats[0].messages;
        assert_eq!(msgs.len(), 1, "stale assistant reply should be popped");
        assert!(matches!(msgs[0].role, Role::User));
    });
}

/// A backend that streams two plan snapshots, then finishes — the card
/// must update in place as each `Plan` event lands.
struct PlanBackend;

impl AgentBackend for PlanBackend {
    fn name(&self) -> &'static str {
        "plan"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        let step = |id: usize, label: &str, status: PlanStatus| crate::model::PlanStep { id, label: label.into(), status };
        for e in [
            AgentEvent::Plan {
                ix: 7,
                steps: vec![step(0, "scan repo", PlanStatus::InProgress), step(1, "edit files", PlanStatus::Pending)],
            },
            AgentEvent::Plan {
                ix: 7,
                steps: vec![step(0, "scan repo", PlanStatus::Done), step(1, "edit files", PlanStatus::InProgress)],
            },
            AgentEvent::Done,
        ] {
            let _ = tx.send(e);
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// A streamed plan renders as a checklist card whose steps update in place
/// as later `Plan` snapshots arrive.
#[test]
fn plan_card_renders_checklist_and_updates() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(PlanBackend);
            this.composer.update(cx, |composer, cx| composer.set_value("go", window, cx));
            this.send(window, cx);
        });
    });
    for _ in 0..8 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
    }
    // One plan message, holding the second snapshot.
    ws.read_with(cx, |ws, _| {
        let plans: Vec<_> = ws.chats[0]
            .messages
            .iter()
            .filter_map(|m| match &m.kind {
                MessageKind::Plan(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(plans.len(), 1, "both Plan events must land on one card");
        assert_eq!(plans[0].steps[0].status, PlanStatus::Done);
        assert_eq!(plans[0].steps[1].status, PlanStatus::InProgress);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("plan", 1usize)).visible(), "plan card header should render");
        // Step rows expose checkbox state: done → checked, in-progress
        // → indeterminate, pending → unchecked.
        let done = window.find("plan-step-1-0");
        assert_eq!(done.checked(), Some(true), "done step must read checked");
        let wip = window.find("plan-step-1-1");
        assert_eq!(wip.indeterminate(), Some(true), "in-progress step must read indeterminate");
        assert_eq!(wip.label(), Some("edit files"));
    });
}

/// Right-click on a message opens the context menu with the copy variants
/// grouped on top; "Copy as Markdown" writes the raw source.
#[test]
fn context_menu_lists_copy_variants() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| this.push_note("**bold** reply".into(), cx));
    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        let mut labels: Vec<String> = snapshots(window)
            .iter()
            .filter(|s| s.role() == Some(A11yRole::MenuItem))
            .filter_map(|s| s.label().map(str::to_string))
            .collect();
        labels.sort();
        // The copy variants group at the top of the menu; no fenced blocks
        // in this message, so Copy Code stays hidden.
        assert_eq!(
            labels,
            ["Bookmark", "Copy", "Copy as Markdown", "Fork here", "Quote", "Retry", "View raw"],
            "menu should list the copy variants: {labels:?}"
        );
        window.within("popup-menu").click(1usize, cx); // Copy as Markdown
    });
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    assert_eq!(clip, "**bold** reply", "Copy as Markdown should write the raw source");
}

/// A message with a fenced block also lists "Copy Code", which writes the
/// block contents without the fences.
#[test]
fn context_menu_copy_code_writes_block_contents() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| this.push_note("try:\n\n```rust\nfn main() {}\n```".into(), cx));
    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let mut labels: Vec<String> = snapshots(window)
            .iter()
            .filter(|s| s.role() == Some(A11yRole::MenuItem))
            .filter_map(|s| s.label().map(str::to_string))
            .collect();
        labels.sort();
        assert_eq!(
            labels,
            ["Bookmark", "Copy", "Copy Code", "Copy as Markdown", "Fork here", "Quote", "Retry", "View raw"],
            "Copy Code should join the copy group: {labels:?}"
        );
        window.within("popup-menu").click(2usize, cx); // Copy Code
    });
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    assert_eq!(clip, "fn main() {}", "Copy Code should write the block contents");
}
