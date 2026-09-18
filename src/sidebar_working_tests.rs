//! Tests for the sidebar's "working" aggregate (`chat_working`): a chat
//! counts as busy while its own reply streams OR an agent attributed to it
//! (`Agent.chat_id`) is still Running — covering the gap where a turn's
//! reply ended but its subagents haven't. The row spinner, the Running
//! filter chip and `running_chats` all read the same aggregate so they
//! can't disagree. Same mount harness as `sidebar_filter_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{Agent, AgentStatus, Chat};
use crate::sidebar_filter::SidebarFilter;
use crate::workspace::Workspace;

fn idle_chat() -> Chat {
    Chat::new(0, "chat")
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-sidebar-working-test-{}", std::process::id()));
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

/// Advance the test clock until no agent is Running (or the budget runs
/// out) — same pump as `agents_tests::settle_agents`.
fn settle_agents(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| ws.agents.iter().all(|a| a.status != AgentStatus::Running)) {
            return;
        }
    }
    panic!("agents never finished");
}

/// The Running chip reads the same aggregate as the row spinner: an idle
/// chat with a Running agent attributed to it matches — settled,
/// unattributed or other-chat agents don't count.
#[test]
fn running_chip_counts_attributed_agents() {
    let idle = idle_chat();
    let mut sub = Agent::new(1, "sub", "sim", 2);
    sub.chat_id = Some(idle.id);
    assert!(SidebarFilter::Running.matches(&idle, std::slice::from_ref(&sub)), "attributed running agent matches");

    // Attribution is per chat — another chat's agent doesn't count.
    let mut other = Agent::new(2, "other", "sim", 2);
    other.chat_id = Some(999);
    assert!(!SidebarFilter::Running.matches(&idle, std::slice::from_ref(&other)));

    // Unattributed agents count on no chat.
    assert!(!SidebarFilter::Running.matches(&idle, &[Agent::new(3, "stray", "sim", 2)]));

    // A settled agent releases the chat.
    sub.status = AgentStatus::Done;
    assert!(!SidebarFilter::Running.matches(&idle, &[sub]));

    // The chat's own reply still qualifies on its own.
    let mut running = idle_chat();
    running.running = true;
    assert!(SidebarFilter::Running.matches(&running, &[]));
}

/// `chat_working`, `running_chats` and the Running chip all count a chat
/// while an agent attributed to it still runs. `chat.running` stays clear:
/// the aggregate lives beside it, not in it.
#[test]
fn working_covers_attributed_agents() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            let (busy, other) = (this.chats[0].id, this.chats[1].id);
            let spec = |name, steps_total, chat_id| crate::simulate::AgentSpec { name, lane: "sim", steps_total, chat_id };
            this.spawn_agent(spec("sub", 1, Some(busy)), cx);
            this.spawn_agent(spec("long", 3, Some(other)), cx);
            // Unattributed agents count on no chat.
            this.spawn_agent(spec("stray", 3, None), cx);

            assert!(!this.chats[0].running && !this.chats[1].running, "no reply is streaming");
            assert!(this.chat_working(&this.chats[0]), "attributed agent keeps the chat working");
            assert!(this.chat_working(&this.chats[1]));
            assert_eq!(this.running_chats(), 2, "the stray agent counts on no chat");

            this.sidebar_filters.toggle(SidebarFilter::Running);
            assert_eq!(this.sidebar_visible(""), vec![1, 0], "Running chip lists agent-busy chats");
        });
    });

    // The one-step agent settles first — its chat goes idle while the
    // longer agent keeps `other` working.
    cx.executor().advance_clock(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    cx.update(|_, cx| {
        ws.update(cx, |this, _cx| {
            assert!(!this.chat_working(&this.chats[0]), "settled agent releases the chat");
            assert!(this.chat_working(&this.chats[1]), "the longer agent still counts");
            assert_eq!(this.running_chats(), 1);
            assert_eq!(this.sidebar_visible(""), vec![1]);
        });
    });

    // When the last attributed agent settles nothing counts as working.
    cx.executor().advance_clock(std::time::Duration::from_millis(1600));
    cx.run_until_parked();
    cx.update(|_, cx| {
        ws.update(cx, |this, _cx| {
            assert_eq!(this.running_chats(), 0);
            assert_eq!(this.sidebar_visible(""), Vec::<usize>::new(), "Running chip empties once all agents settle");
        });
    });
}

/// A task agent spawned from the panel's input row attributes itself to
/// the chat that was active at spawn — that session stays "working" even
/// after the user switches away. The attribution doesn't follow the
/// switch: it's stamped once, on the spawning chat.
#[test]
fn task_agent_marks_its_spawning_chat_working() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(crate::backend::SimBackend);
            this.new_chat(cx);
            this.spawn_task_agent("fix the flaky test".to_string(), cx);
            let spawned = this.chats[this.active].id;
            assert_eq!(this.agents[0].chat_id, Some(spawned), "task agent attributes to the active chat");
            assert!(this.chat_working(&this.chats[this.active]), "spawning chat counts as working");

            // Switching chats leaves the attribution behind — the spawning
            // session stays busy, not the newly-active one.
            this.select_chat(0, window, cx);
            assert!(!this.chat_working(&this.chats[0]), "the newly-active chat isn't working");
            assert!(this.chat_working(&this.chats[1]), "the spawning chat still is");
        });
    });

    // The sim stream drains in a few polls — the marker lifts when the
    // task agent finishes.
    settle_agents(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _cx| {
            assert_eq!(this.running_chats(), 0, "nothing counts as working once the task finishes");
        });
    });
}
