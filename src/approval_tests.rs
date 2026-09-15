//! Headless tests for the approval-prompt flow: a fake backend emits
//! `AgentEvent::ApprovalRequest` and blocks on the responder, the UI card
//! renders, and clicking a button must deliver the decision back to the
//! waiting backend thread.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ApprovalDecision, ApprovalKind, ReplyStream, TurnContext};
use crate::model::MessageKind;
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats. Each mount gets a fresh
/// dir — a second workspace in one process must not reload the first's
/// persisted chats.
fn sandbox_home() {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("rixlcode-approval-test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
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

/// A backend that emits one approval request, blocks on the responder,
/// forwards the decision to the test, then finishes the turn.
struct ApprovalBackend {
    decisions: std::sync::mpsc::Sender<ApprovalDecision>,
}

impl AgentBackend for ApprovalBackend {
    fn name(&self) -> &'static str {
        "approval-fake"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        let (respond, rx) = std::sync::mpsc::channel();
        let decisions = self.decisions.clone();
        std::thread::spawn(move || {
            let _ = tx.send(AgentEvent::ApprovalRequest {
                ix: 7,
                kind: ApprovalKind::Command,
                detail: "rm -rf build/".into(),
                respond,
            });
            // Blocked until the card answers — or the responder drops.
            let decision = rx.recv().unwrap_or(ApprovalDecision::Deny);
            let _ = decisions.send(decision);
            let _ = tx.send(AgentEvent::Done);
        });
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Send "hi" on the fake backend and pump the reply task's 30ms poll loop
/// until the approval card lands in the chat.
fn send_and_wait_card(
    ws: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    backend: std::sync::Arc<dyn AgentBackend>,
) {
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = backend;
            this.model = "gpt-5".into();
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    // The backend thread runs on real time while the reply task's 30ms
    // poll runs on the fake clock — alternate real sleeps with clock
    // advances until the card lands (or a real-time deadline trips).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        let landed = ws.read_with(cx, |ws, _| {
            ws.chats[0].messages.iter().any(|m| matches!(m.kind, MessageKind::Approval(_)))
        });
        if landed {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "approval card never landed");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The approval card is message 1 (the user's "hi" is message 0).
fn card(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> crate::backend::ApprovalCard {
    ws.read_with(cx, |ws, _| {
        ws.chats[0]
            .messages
            .iter()
            .find_map(|m| match &m.kind {
                MessageKind::Approval(a) => Some(a.clone()),
                _ => None,
            })
            .expect("approval card message")
    })
}

#[test]
fn approval_card_renders_and_approve_replies() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, std::sync::Arc::new(ApprovalBackend { decisions: decisions_tx }));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The card shows the gated command and all three buttons.
        assert!(window.find(("approval", 1usize)).visible(), "approval card renders");
        assert!(window.find(("approval-detail", 1usize)).visible(), "command detail renders");
        for id in ["approve", "deny", "always"] {
            assert!(window.find((id, 1usize)).visible(), "{id} button renders");
        }
        window.click(("approve", 1usize), cx);
    });

    // The click reached the blocked backend thread.
    let decision = decisions.recv_timeout(std::time::Duration::from_secs(5)).expect("backend got a decision");
    assert_eq!(decision, ApprovalDecision::Approve);
    assert_eq!(card(&ws, cx).decision, Some(ApprovalDecision::Approve));

    // Done arrived — the card now shows the outcome instead of buttons.
    for _ in 0..8 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("approval-outcome", 1usize)).visible(), "answered card shows the outcome");
        assert!(window.try_find(("approve", 1usize)).is_none(), "buttons collapse after answering");
    });
}

/// Click a button on the rendered card and return the decision the
/// backend's blocked thread received.
fn click_and_collect(
    cx: &mut VisualTestContext,
    button: &'static str,
    decisions: std::sync::mpsc::Receiver<ApprovalDecision>,
) -> ApprovalDecision {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click((button, 1usize), cx);
    });
    decisions.recv_timeout(std::time::Duration::from_secs(5)).expect("backend got a decision")
}

#[test]
fn deny_button_sends_deny() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, std::sync::Arc::new(ApprovalBackend { decisions: decisions_tx }));

    let decision = click_and_collect(cx, "deny", decisions);
    assert_eq!(decision, ApprovalDecision::Deny);
    assert_eq!(card(&ws, cx).decision, Some(ApprovalDecision::Deny));
}

#[test]
fn always_button_sends_approve_for_session() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, std::sync::Arc::new(ApprovalBackend { decisions: decisions_tx }));

    let decision = click_and_collect(cx, "always", decisions);
    assert_eq!(decision, ApprovalDecision::ApproveForSession);
    assert_eq!(card(&ws, cx).decision, Some(ApprovalDecision::ApproveForSession));
}

#[test]
fn stop_reply_denies_a_pending_approval() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, std::sync::Arc::new(ApprovalBackend { decisions: decisions_tx }));

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.stop_reply(cx));
    });
    // Stopping the turn answers Deny so the backend's blocked read unblocks.
    let decision = decisions.recv_timeout(std::time::Duration::from_secs(5)).expect("backend got a decision");
    assert_eq!(decision, ApprovalDecision::Deny);
    assert_eq!(card(&ws, cx).decision, Some(ApprovalDecision::Deny));
}
