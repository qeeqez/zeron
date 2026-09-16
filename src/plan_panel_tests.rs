//! Tests for the Plan panel: `Chat::latest_plan` extraction, the done/total
//! progress count, in-place `update_plan` refreshes, the persisted open flag,
//! and headless toggle/render coverage. Same harness as `ui_tests.rs` —
//! plain `#[test]` + narrow imports (a bare `use gpui_kit::*` shadows the
//! built-in `#[test]` the `#[gpui_kit::test]` macro relies on).

use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, KeyBinding, TestAppContext, VisualTestContext};

use crate::model::{Chat, ChatMessage, MessageKind, PlanCard, PlanStatus, PlanStep, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-plan-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

fn step(id: usize, label: &str, status: PlanStatus) -> PlanStep {
    PlanStep { id, label: label.into(), status }
}

fn plan_msg(plan_ix: usize, steps: Vec<PlanStep>) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Plan(PlanCard { plan_ix, steps }),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn text_msg(text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

#[test]
fn latest_plan_returns_last_plan_card() {
    let mut chat = Chat::new(0, "t");
    assert!(chat.latest_plan().is_none(), "no plan messages → no plan");
    Rc::make_mut(&mut chat.messages).push(plan_msg(0, vec![step(0, "first", PlanStatus::Pending)]));
    Rc::make_mut(&mut chat.messages).push(text_msg("working on it"));
    Rc::make_mut(&mut chat.messages).push(plan_msg(1, vec![step(0, "second", PlanStatus::Done)]));
    let plan = chat.latest_plan().expect("a plan card exists");
    assert_eq!(plan.plan_ix, 1, "the later card wins even with trailing text");
    assert_eq!(plan.steps[0].label.as_str(), "second");
}

#[test]
fn plan_progress_counts_done_steps() {
    let plan = PlanCard {
        plan_ix: 0,
        steps: vec![
            step(0, "a", PlanStatus::Done),
            step(1, "b", PlanStatus::InProgress),
            step(2, "c", PlanStatus::Pending),
            step(3, "d", PlanStatus::Done),
        ],
    };
    assert_eq!(plan.done_count(), 2);
    assert_eq!(plan.steps.len(), 4);
}

/// `apply_plan` rewrites a card's steps in place — `latest_plan` must see
/// the new snapshot, not the one the card was appended with.
#[test]
fn in_place_step_updates_change_the_latest_plan() {
    let mut chat = Chat::new(0, "t");
    Rc::make_mut(&mut chat.messages).push(plan_msg(0, vec![step(0, "a", PlanStatus::Pending), step(1, "b", PlanStatus::Pending)]));
    if let MessageKind::Plan(p) = &mut Rc::make_mut(&mut chat.messages)[0].kind {
        p.steps = vec![step(0, "a", PlanStatus::Done), step(1, "b", PlanStatus::InProgress)];
    }
    let plan = chat.latest_plan().unwrap();
    assert_eq!(plan.done_count(), 1);
    assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
}

#[test]
fn plan_panel_toggles_via_keybinding_and_close_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([KeyBinding::new("cmd-shift-p", crate::TogglePlan, Some("workspace"))]);
        window.draw(cx).clear(cx);
        assert!(window.try_find("plan-panel").is_none(), "panel starts closed");

        window.press("cmd-shift-p", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("plan-panel").visible(), "cmd-shift-p opens the panel");
        assert!(ws.read(cx).plan_panel.open);
        assert!(window.find("plan-empty").visible(), "no plan → empty state");

        window.click("close-plan", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("plan-panel").is_none(), "close button hides the panel");
        assert!(!ws.read(cx).plan_panel.open);
    });
}

#[test]
fn plan_panel_lists_steps_and_updates_live() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            Rc::make_mut(&mut this.chats[this.active].messages)
                .push(plan_msg(0, vec![step(0, "scan repo", PlanStatus::Done), step(1, "fix bug", PlanStatus::InProgress)]));
            this.plan_panel.open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("plan-progress").label(), Some("1/2"), "progress count renders");
        let done = window.find(("plan-panel-step", 0usize));
        assert_eq!(done.label(), Some("scan repo"));
        assert_eq!(done.checked(), Some(true), "done step reads checked");
        let active = window.find(("plan-panel-step", 1usize));
        assert_eq!(active.label(), Some("fix bug"));
        assert_eq!(active.indeterminate(), Some(true), "in-progress step reads mixed");

        // An update_plan snapshot lands: steps rewrite in place and the
        // panel's next frame shows the new status + count.
        ws.update(cx, |this, cx| {
            if let MessageKind::Plan(p) = &mut Rc::make_mut(&mut this.chats[this.active].messages)[0].kind {
                p.steps = vec![step(0, "scan repo", PlanStatus::Done), step(1, "fix bug", PlanStatus::Done)];
            }
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("plan-progress").label(), Some("2/2"), "progress count updates live");
        assert_eq!(window.find(("plan-panel-step", 1usize)).checked(), Some(true), "step flips to done");
    });
}

/// The open flag round-trips through settings.json: toggling writes it, a
/// fresh workspace restores it.
#[test]
fn plan_panel_open_state_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.toggle_plan_panel(cx));
    });
    assert!(crate::persist::load_settings().plan_panel_open, "toggle writes the setting");

    // A second workspace on the same HOME restores the persisted flag.
    let (ws2, cx2) = app.add_window_view(Workspace::new);
    ws2.read_with(cx2, |ws, _| {
        assert!(ws.plan_panel.open, "panel reopens on launch");
    });
}
