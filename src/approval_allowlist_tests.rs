//! Headless tests for the durable approval allowlist: an approval card's
//! "Always allow" records a per-project rule in `state.json`, a later
//! request whose signature matches auto-approves without prompting, and
//! deleting the rule restores the prompt.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ApprovalDecision, ApprovalKind, ApprovalRule, ReplyStream, TurnContext};
use crate::model::MessageKind;
use crate::project::{Project, ProjectState};
use crate::workspace::Workspace;

/// A fresh temp dir (HOME and project roots both live under it).
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-allowlist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mount a `Workspace` bound to `project` — HOME must already point at the
/// test's temp dir.
fn mount_at(cx: &mut TestAppContext, project: Project) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::for_project(project, window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// `mount_at` with `HOME` redirected to a fresh temp dir first.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", temp_dir(name)) };
    mount_at(cx, project)
}

/// A backend that emits one approval request, blocks on the responder,
/// forwards the decision to the test, then finishes the turn.
struct ApprovalBackend {
    decisions: std::sync::mpsc::Sender<ApprovalDecision>,
    kind: ApprovalKind,
    detail: &'static str,
}

impl AgentBackend for ApprovalBackend {
    fn name(&self) -> &'static str {
        "allowlist-fake"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        let (tx, events) = std::sync::mpsc::channel();
        let (respond, rx) = std::sync::mpsc::channel();
        let decisions = self.decisions.clone();
        let (kind, detail) = (self.kind, self.detail);
        std::thread::spawn(move || {
            let _ = tx.send(AgentEvent::ApprovalRequest { ix: 7, kind, detail: detail.into(), respond });
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
fn send_and_wait_card(ws: &Entity<Workspace>, cx: &mut VisualTestContext, backend: std::sync::Arc<dyn AgentBackend>) {
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
        let landed = ws.read_with(cx, |ws, _| ws.chats[0].messages.iter().any(|m| matches!(m.kind, MessageKind::Approval(_))));
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

/// Poll until the card's `button` registers in the element tree — the
/// card lands in model state a beat before its buttons mount, so a single
/// draw races the mount under parallel test load.
fn wait_button(cx: &mut VisualTestContext, button: &'static str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.try_find((button, 1usize)).is_some_and(|b| b.visible())
    }) {
        assert!(std::time::Instant::now() < deadline, "{button} button never rendered");
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Click a button on the rendered card and return the decision the
/// backend's blocked thread received.
fn click_and_collect(
    cx: &mut VisualTestContext, button: &'static str, decisions: std::sync::mpsc::Receiver<ApprovalDecision>,
) -> ApprovalDecision {
    wait_button(cx, button);
    cx.update(|window, cx| window.click((button, 1usize), cx));
    decisions.recv_timeout(std::time::Duration::from_secs(5)).expect("backend got a decision")
}

fn backend(
    decisions: std::sync::mpsc::Sender<ApprovalDecision>, kind: ApprovalKind, detail: &'static str,
) -> std::sync::Arc<dyn AgentBackend> {
    std::sync::Arc::new(ApprovalBackend { decisions, kind, detail })
}

#[test]
fn always_allow_records_a_durable_rule() {
    let root = temp_dir("record");
    let project = Project::open(&root);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "record", project.clone());
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, backend(decisions_tx, ApprovalKind::Command, "rm -rf build/"));

    let decision = click_and_collect(cx, "always", decisions);
    assert_eq!(decision, ApprovalDecision::ApproveForSession);
    let rule = ApprovalRule::for_prompt(ApprovalKind::Command, "rm -rf build/");
    assert_eq!(ws.read_with(cx, |ws, _| ws.approval_rules.clone()), vec![rule.clone()]);
    assert_eq!(project.load_state().approval_rules, vec![rule], "the rule persists to state.json");
    assert!(!card(&ws, cx).auto_approved, "a clicked card is not auto-approved");
}

#[test]
fn matching_rule_auto_approves_without_prompting() {
    let root = temp_dir("auto");
    let project = Project::open(&root);
    project.save_state(&ProjectState {
        approval_rules: vec![ApprovalRule::for_prompt(ApprovalKind::Command, "rm -rf build/")],
        ..Default::default()
    });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "auto", project);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, backend(decisions_tx, ApprovalKind::Command, "rm -rf build/"));

    // The blocked backend got its Approve without a click.
    let decision = decisions.recv_timeout(std::time::Duration::from_secs(5)).expect("backend got a decision");
    assert_eq!(decision, ApprovalDecision::Approve);
    let card = card(&ws, cx);
    assert_eq!(card.decision, Some(ApprovalDecision::Approve));
    assert!(card.auto_approved);
    assert!(card.respond.is_none(), "an answered card holds no responder");

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(
            window.find(("approval-outcome", 1usize)).label(),
            Some("Auto-approved · rule"),
            "the card renders the auto-approved label"
        );
        for id in ["approve", "deny", "always"] {
            assert!(window.try_find((id, 1usize)).is_none(), "{id} button never renders");
        }
    });
}

#[test]
fn non_matching_rule_still_prompts() {
    let root = temp_dir("nomatch");
    let project = Project::open(&root);
    project.save_state(&ProjectState {
        approval_rules: vec![ApprovalRule::for_prompt(ApprovalKind::Command, "npm test")],
        ..Default::default()
    });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "nomatch", project);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, backend(decisions_tx, ApprovalKind::Command, "rm -rf build/"));

    let card = card(&ws, cx);
    assert_eq!(card.decision, None, "no rule matched — the prompt waits");
    assert!(card.respond.is_some());
    wait_button(cx, "approve");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("approve", 1usize)).visible(), "buttons render for an unmatched request");
    });
    let decision = click_and_collect(cx, "approve", decisions);
    assert_eq!(decision, ApprovalDecision::Approve);
}

#[test]
fn rule_kind_is_part_of_the_signature() {
    let root = temp_dir("kind");
    let project = Project::open(&root);
    project.save_state(&ProjectState {
        approval_rules: vec![ApprovalRule::for_prompt(ApprovalKind::Command, "rm -rf build/")],
        ..Default::default()
    });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "kind", project);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    // Same detail, different kind — a command grant must not cover a
    // permission request.
    send_and_wait_card(&ws, cx, backend(decisions_tx, ApprovalKind::Permission, "rm -rf build/"));

    let card = card(&ws, cx);
    assert_eq!(card.decision, None, "a command rule must not match a permission request");
    let decision = click_and_collect(cx, "approve", decisions);
    assert_eq!(decision, ApprovalDecision::Approve);
}

#[test]
fn rule_delete_restores_the_prompt() {
    let root = temp_dir("delete");
    let project = Project::open(&root);
    project.save_state(&ProjectState {
        approval_rules: vec![ApprovalRule::for_prompt(ApprovalKind::Command, "rm -rf build/")],
        ..Default::default()
    });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "delete", project.clone());
    assert_eq!(ws.read_with(cx, |ws, _| ws.approval_rules.len()), 1);

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.remove_approval_rule(0, cx));
    });
    assert!(ws.read_with(cx, |ws, _| ws.approval_rules.is_empty()));
    assert!(project.load_state().approval_rules.is_empty(), "the delete persists to state.json");

    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, backend(decisions_tx, ApprovalKind::Command, "rm -rf build/"));
    let card = card(&ws, cx);
    assert_eq!(card.decision, None, "the deleted rule no longer auto-approves");
    assert!(card.respond.is_some());
    let decision = click_and_collect(cx, "approve", decisions);
    assert_eq!(decision, ApprovalDecision::Approve);
}

#[test]
fn whitespace_variants_share_one_rule() {
    let root = temp_dir("whitespace");
    let project = Project::open(&root);
    // The stored rule's spacing differs from the request's — normalization
    // makes them the same signature.
    project.save_state(&ProjectState {
        approval_rules: vec![ApprovalRule::for_prompt(ApprovalKind::Command, "  rm   -rf  build/ ")],
        ..Default::default()
    });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "whitespace", project);
    let (decisions_tx, decisions) = std::sync::mpsc::channel();
    send_and_wait_card(&ws, cx, backend(decisions_tx, ApprovalKind::Command, "rm -rf build/"));

    let decision = decisions.recv_timeout(std::time::Duration::from_secs(5)).expect("backend got a decision");
    assert_eq!(decision, ApprovalDecision::Approve);
    assert!(card(&ws, cx).auto_approved);
}
