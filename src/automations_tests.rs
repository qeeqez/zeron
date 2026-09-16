//! Scheduled-prompt tests: the due/skip math on `Automation`, persistence
//! round-trips, and the workspace-level fire path — a due automation sends
//! its prompt through the real turn pipeline (sim backend), a busy chat
//! skips the occurrence, and a deleted chat drops its automation.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use std::time::{Duration, SystemTime};

use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::automations::{Automation, AutomationInterval};
use crate::composer_testutil::{open_workspace, until, use_sim};
use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

fn automation(chat_id: u64, interval: AutomationInterval, next_run: SystemTime) -> Automation {
    Automation {
        id: 1,
        chat_id,
        prompt: "check the build".to_string(),
        interval,
        enabled: true,
        next_run,
        last_run: None,
    }
}

#[test]
fn due_requires_enabled_and_past_next_run() {
    let now = SystemTime::now();
    let a = automation(7, AutomationInterval::H1, now);
    assert!(a.due(now), "next_run == now is due");
    assert!(a.due(now + Duration::from_secs(1)), "past next_run is due");
    assert!(!a.due(now - Duration::from_secs(1)), "future next_run is not due");
    let mut disabled = a.clone();
    disabled.enabled = false;
    assert!(!disabled.due(now + Duration::from_secs(3600)), "disabled never fires");
}

#[test]
fn mark_fired_advances_one_interval_from_now() {
    // A next_run far in the past — the app was closed for several slots —
    // still schedules exactly one interval out: catch-up runs once, never
    // once per missed slot.
    let now = SystemTime::now();
    let mut a = automation(7, AutomationInterval::M15, now - Duration::from_secs(3600));
    a.mark_fired(now);
    assert_eq!(a.last_run, Some(now));
    assert_eq!(a.next_run, now + Duration::from_secs(900));
    assert!(!a.due(now), "just-fired isn't due again");
}

#[test]
fn skip_reschedules_without_touching_last_run() {
    let now = SystemTime::now();
    let mut a = automation(7, AutomationInterval::H6, now - Duration::from_secs(60));
    a.skip(now);
    assert_eq!(a.last_run, None, "a skipped occurrence isn't a run");
    assert_eq!(a.next_run, now + Duration::from_secs(6 * 3600));
}

#[test]
fn persistence_round_trip() {
    let dir = std::env::temp_dir().join(format!("rixlcode-automations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = automation(42, AutomationInterval::H24, SystemTime::now() + Duration::from_secs(60));
    a.id = 42;
    a.last_run = Some(SystemTime::now());
    crate::persist::save_automations(&dir, &[a.clone()]);
    let loaded = crate::persist::load_automations(&dir);
    assert_eq!(loaded.len(), 1);
    let b = &loaded[0];
    assert_eq!((b.id, b.chat_id, b.prompt.as_str(), b.interval, b.enabled), (42, 42, "check the build", AutomationInterval::H24, true));
    assert_eq!(b.next_run, a.next_run);
    assert_eq!(b.last_run, a.last_run);
    // An empty store removes the file — a cleared schedule stays cleared.
    crate::persist::save_automations(&dir, &[]);
    assert!(crate::persist::load_automations(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Add `a` to the workspace, keeping the id counter consistent.
fn add(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, a: Automation) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.next_automation_id = ws.next_automation_id.max(a.id + 1);
            ws.automations.push(a);
        });
    });
}

/// The chat's user messages containing `needle`.
fn user_msgs_in(ws: &Workspace, chat_id: u64, needle: &str) -> usize {
    ws.chats
        .iter()
        .find(|c| c.id == chat_id)
        .map(|c| {
            c.messages
                .iter()
                .filter(|m| m.role == Role::User && matches!(&m.kind, MessageKind::Text(t) if t.contains(needle)))
                .count()
        })
        .unwrap_or(0)
}

#[gpui_kit::test]
fn due_automation_fires_into_its_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    // A second chat so the automation's target isn't the active one —
    // the fire must not steal the selection.
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.new_chat(cx));
        workspace.update(cx, |ws, cx| ws.select_chat(0, window, cx));
    });
    let (target_id, active_id) = workspace.read_with(cx, |ws, _| (ws.chats[1].id, ws.chats[ws.active].id));
    // The target chat's stamps would route it to the real backend — clear
    // them so the turn follows the workspace's sim backend.
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            let chat = ws.chats.iter_mut().find(|c| c.id == target_id).unwrap();
            chat.provider.clear();
            chat.model.clear();
        });
    });
    add(&workspace, cx, automation(target_id, AutomationInterval::M15, SystemTime::now() - Duration::from_secs(1)));
    // The 1s ticker fires the automation; the sim reply streams in.
    until(&workspace, cx, |ws| {
        user_msgs_in(ws, target_id, "check the build") == 1 && ws.chats.iter().any(|c| c.id == target_id && !c.running)
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.chats[ws.active].id, active_id, "the fire must not switch chats");
        let chat = ws.chats.iter().find(|c| c.id == target_id).unwrap();
        assert!(chat.unread, "a background chat's scheduled turn marks it unread");
        assert!(chat.messages.iter().any(|m| m.role == Role::Assistant), "the reply streamed in");
        let a = &ws.automations[0];
        assert!(a.last_run.is_some());
        assert!(a.next_run > SystemTime::now(), "next run moved one interval out");
    });
}

#[gpui_kit::test]
fn running_chat_skips_the_occurrence(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    let chat_id = workspace.read_with(cx, |ws, _| ws.chats[ws.active].id);
    // Mid-turn: the automation's slot passes while the chat is busy.
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| ws.chats.iter_mut().find(|c| c.id == chat_id).unwrap().running = true);
    });
    let mut a = automation(chat_id, AutomationInterval::M15, SystemTime::now() - Duration::from_secs(1));
    a.id = 9;
    add(&workspace, cx, a);
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        let a = &ws.automations[0];
        assert_eq!(user_msgs_in(ws, chat_id, "check the build"), 0, "a busy chat skips the send");
        assert_eq!(a.last_run, None, "a skipped occurrence isn't a run");
        assert!(a.next_run > SystemTime::now(), "the occurrence rescheduled one interval out");
    });
}

#[gpui_kit::test]
fn deleted_chat_drops_its_automation(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    let mut a = automation(u64::MAX, AutomationInterval::H1, SystemTime::now() - Duration::from_secs(1));
    a.id = 4;
    add(&workspace, cx, a);
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.fire_due_automations(window, cx));
    });
    workspace.read_with(cx, |ws, _| {
        assert!(ws.automations.is_empty(), "an automation whose chat is gone is dropped");
    });
}

#[gpui_kit::test]
fn toggle_and_delete_persist(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    let chat_id = workspace.read_with(cx, |ws, _| ws.chats[ws.active].id);
    let mut a = automation(chat_id, AutomationInterval::H1, SystemTime::now() + Duration::from_secs(3600));
    a.id = 3;
    add(&workspace, cx, a);
    cx.update(|_, cx| {
        workspace.update(cx, |ws, cx| ws.toggle_automation(3, false, cx));
    });
    workspace.read_with(cx, |ws, _| {
        assert!(!ws.automations[0].enabled);
        assert!(!ws.automations[0].due(SystemTime::now() + Duration::from_secs(86400 * 7)), "disabled stays off");
    });
    // The toggle wrote the file — reload proves the round-trip.
    let dir = workspace.read_with(cx, |ws, _| ws.project.dir().to_path_buf());
    let loaded = crate::persist::load_automations(&dir);
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].enabled);
    cx.update(|_, cx| {
        workspace.update(cx, |ws, cx| ws.delete_automation(3, cx));
    });
    workspace.read_with(cx, |ws, _| assert!(ws.automations.is_empty()));
    assert!(crate::persist::load_automations(&dir).is_empty(), "delete removes the file");
}
