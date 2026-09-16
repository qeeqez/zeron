//! Tests for the dock badge: the count/dedupe state machine as unit tests,
//! plus headless `Workspace` runs for the unread → badge → read → clear
//! flow and the focus clear wired in `lifecycle::open_workspace_window_for`.
//! The AppKit write itself is macOS-only and untestable headless — tests
//! assert `last_badge`, the dedupe state that gates it.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
use gpui_kit::TestAppContext;

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::composer_testutil::{open_workspace, until};
use crate::dock_badge::{self, BadgeChange};
use crate::model::Chat;

#[test]
fn counts_unread_chats() {
    let mut chats = vec![Chat::new(1, "a"), Chat::new(2, "b"), Chat::new(3, "c")];
    assert_eq!(dock_badge::unread_count(&chats), 0, "fresh chats are all read");
    chats[0].unread = true;
    chats[2].unread = true;
    assert_eq!(dock_badge::unread_count(&chats), 2);
    chats[0].unread = false;
    chats[2].unread = false;
    assert_eq!(dock_badge::unread_count(&chats), 0, "all read clears the count");
}

#[test]
fn dedupes_unchanged_labels() {
    let mut last = None;
    assert_eq!(dock_badge::transition(&mut last, 0), BadgeChange::Unchanged, "never badged stays clear");
    assert_eq!(dock_badge::transition(&mut last, 3), BadgeChange::Set(3));
    assert_eq!(dock_badge::transition(&mut last, 3), BadgeChange::Unchanged, "same count doesn't re-set");
    assert_eq!(dock_badge::transition(&mut last, 5), BadgeChange::Set(5));
    assert_eq!(dock_badge::transition(&mut last, 0), BadgeChange::Clear);
    assert_eq!(dock_badge::transition(&mut last, 0), BadgeChange::Unchanged, "already clear stays clear");
}

/// A backend whose turn completes immediately — deterministic, unlike
/// `SimBackend`, which fails a quarter of replies at random.
struct OkBackend;

impl AgentBackend for OkBackend {
    fn name(&self) -> &'static str {
        "ok"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta("done".into()));
        let _ = tx.send(AgentEvent::Done);
        drop(tx);
        ReplyStream {
            events: rx,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

#[gpui_kit::test]
fn unread_reply_badges_then_selecting_clears(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.backend = std::sync::Arc::new(OkBackend);
            ws.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            ws.send(window, cx);
            // Switch away before the reply lands — a finished turn on a
            // background chat is what flags it unread.
            ws.new_chat(cx);
        });
    });
    until(&workspace, cx, |ws| ws.chats[0].unread);
    cx.run_until_parked();
    assert_eq!(dock_badge::last_badge(), Some(1), "one unread chat badges the dock");

    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.select_chat(0, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(dock_badge::last_badge(), None, "selecting the unread chat clears the badge");
}

#[gpui_kit::test]
fn focusing_a_window_clears_the_badge(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    // The activation observer lives in `open_workspace_window_for` — the
    // real window path — so open a second window through it.
    cx.update(|_, cx| {
        cx.spawn(async move |cx| {
            let _ = crate::lifecycle::open_workspace_window(cx);
        })
        .detach();
    });
    cx.run_until_parked();
    let handle = cx.update(|_, cx| *cx.windows().last().expect("lifecycle window never opened"));

    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| ws.chats[0].unread = true);
        dock_badge::update(cx);
    });
    cx.run_until_parked();
    assert_eq!(dock_badge::last_badge(), Some(1));

    let _ = handle.update(cx, |_, window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(dock_badge::last_badge(), None, "focusing the app clears the dock badge");
}
