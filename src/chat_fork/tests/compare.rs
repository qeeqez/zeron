//! "Compare providers…" tests: `compare_providers` forks the active chat
//! once per picked instance and sends each the same prompt — the composer
//! draft, or the last user message when the draft is empty — plus the
//! ⋯ menu item's disabled states and the picker dialog's 2+ rule.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, Window};

use super::{mount, seed_transcript};
use crate::model::{MessageKind, ModelInfo};
use crate::providers::{ProviderInstance, ProviderKind};
use crate::workspace::Workspace;

/// A second enabled sim instance — the only kind whose sends run without
/// spawning a real backend. Its catalog is seeded directly (the workspace
/// seeds catalogs at construction, before this instance exists).
fn add_sim2(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, _| {
        let mut p = ProviderInstance::new(ProviderKind::Sim, "Sim Two".into());
        p.id = "sim-2".into();
        this.providers.push(p);
        this.model_catalog
            .insert("sim-2".into(), vec![ModelInfo { id: "sim".into(), label: "Sim".into(), ..Default::default() }]);
    });
}

/// The ⋯ menu on the chat titlebar — opened by clicking the header button.
/// Duplicated from `continue_with.rs` — sibling test files can't share
/// private helpers.
fn open_chat_menu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
    });
}

/// The ⋯ menu's "Compare providers…" row — panics when it isn't offered.
fn compare_menu_item(window: &Window) -> gpui_kit::base::test_support::ElementSnapshot {
    snapshots(window)
        .iter()
        .find(|s| s.label() == Some("Compare providers…"))
        .unwrap_or_else(|| panic!("chat menu should offer Compare providers…"))
        .clone()
}

/// A disabled menu item is observable only as inertness: clicking it opens
/// no dialog and forks nothing.
fn assert_compare_inert(ws: &Entity<Workspace>, cx: &mut VisualTestContext, because: &str) {
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = compare_menu_item(window);
        window.within("popup-menu").click(item.path().last().unwrap().clone(), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("compare-dialog").is_none(), "{because}");
    });
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).chats.len(), 1, "{because}");
    });
}

#[test]
fn compare_forks_once_per_provider_and_sends_the_draft() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    add_sim2(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("same prompt", window, cx));
            this.compare_providers(&["sim".to_string(), "sim-2".to_string()], window, cx);
        });
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 3, "one fork per picked provider");
        let (f1, f2) = (&ws.chats[1], &ws.chats[2]);
        assert_eq!(f1.title.as_ref(), "Fix bug · Sim");
        assert_eq!(f2.title.as_ref(), "Fix bug · Sim Two");
        assert_eq!((f1.provider.as_str(), f2.provider.as_str()), ("sim", "sim-2"), "each fork binds its provider");
        assert_eq!((f1.model.as_str(), f2.model.as_str()), ("sim", "sim"), "each lands on the provider's first model");
        assert!(f1.thread_id.is_empty() && f2.thread_id.is_empty(), "forks start fresh backend threads");
        // The prompt went out on each fork: the transcript copy plus the
        // user message, then the sim turn's tool call.
        for f in [f1, f2] {
            assert!(f.messages.len() >= 5, "fork holds the transcript plus the sent prompt");
            assert!(matches!(&f.messages[4].kind, MessageKind::Text(t) if t.as_ref() == "same prompt"));
            assert!(f.running, "each fork runs its own turn");
        }
        assert_eq!(ws.active, 1, "the first fork is selected");
        assert_eq!(ws.chats[0].messages.len(), 4, "the source transcript is untouched");
        assert_eq!(ws.chats[0].draft, "same prompt", "the draft stays on the source chat");
    });
}

#[test]
fn compare_falls_back_to_the_last_user_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    add_sim2(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.compare_providers(&["sim".to_string(), "sim-2".to_string()], window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 3);
        for f in [&ws.chats[1], &ws.chats[2]] {
            assert!(matches!(&f.messages[4].kind, MessageKind::Text(t) if t.as_ref() == "u2"), "empty draft resends the last user message");
        }
    });
}

#[test]
fn compare_on_an_empty_chat_sends_the_draft() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    add_sim2(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("hello", window, cx));
            this.compare_providers(&["sim".to_string(), "sim-2".to_string()], window, cx);
        });
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 3, "an empty transcript still forks — the draft is the prompt");
        let f = &ws.chats[1];
        assert!(matches!(&f.messages[0].kind, MessageKind::Text(t) if t.as_ref() == "hello"));
        assert!(f.running);
    });
}

#[test]
fn compare_noops_without_two_enabled_picks() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    add_sim2(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.compare_providers(&["sim".to_string()], window, cx);
            this.compare_providers(&["sim".to_string(), "gone".to_string()], window, cx);
            this.compare_providers(&[], window, cx);
        });
    });
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats.len(), 1, "fewer than two enabled picks forks nothing");
    });
}

#[test]
fn compare_noops_while_running_and_without_a_prompt() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    add_sim2(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            // Empty transcript AND empty draft — nothing to send.
            this.compare_providers(&["sim".to_string(), "sim-2".to_string()], window, cx);
            assert_eq!(this.chats.len(), 1, "no prompt, no compare");
            this.chats[0].running = true;
            this.composer.update(cx, |s, cx| s.set_value("later", window, cx));
            this.compare_providers(&["sim".to_string(), "sim-2".to_string()], window, cx);
            assert_eq!(this.chats.len(), 1, "no compare mid-turn");
        });
    });
}

/// The ⋯ menu item opens the dialog; Confirm with one pick stays open and
/// forks nothing, two picks close it and fork + send per provider.
#[test]
fn chat_menu_compare_dialog_drives_the_forks() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    add_sim2(&ws, cx);
    // Only the two sims stay enabled — the sends run simulated, no
    // subprocesses. The chat's stamp (codex-cli, now disabled) isn't in the
    // list, so nothing starts checked.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            for id in ["codex-cli", "claude-cli", "acp", "mcp", "http", "ollama"] {
                this.set_provider_enabled(id, false, cx);
            }
        });
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("pick me", window, cx));
        });
    });
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = compare_menu_item(window);
        assert!(item.disabled() != Some(true), "two enabled providers offer the item");
        window.within("popup-menu").click(item.path().last().unwrap().clone(), cx);
    });
    // The dialog animates in over 250ms of real time — let it settle or
    // the synthetic click lands where the checkbox was, not where it is.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("compare-dialog").visible(), "the item opens the picker dialog");
        // Check the first provider — the toggle lands on the shared picks.
        window.click("compare-check-sim", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("compare-check-sim").checked(), Some(true), "the click checks the row");
        // One pick: Confirm keeps the dialog open and forks nothing.
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
        window.draw(cx).clear(cx);
        assert!(window.find("compare-dialog").visible(), "one pick can't compare");
        // Second pick: Confirm forks once per provider and sends the draft.
        window.click("compare-check-sim-2", cx);
        window.draw(cx).clear(cx);
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_none(), "OK closes the dialog");
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 3, "one fork per pick");
        assert_eq!(ws.chats[1].provider, "sim");
        assert_eq!(ws.chats[2].provider, "sim-2");
        for f in [&ws.chats[1], &ws.chats[2]] {
            assert!(matches!(&f.messages[4].kind, MessageKind::Text(t) if t.as_ref() == "pick me"));
        }
    });
}

#[test]
fn compare_item_disabled_while_running() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| this.chats[0].running = true);
    });
    assert_compare_inert(&ws, cx, "no compare mid-turn");
}

#[test]
fn compare_item_disabled_with_a_single_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            for id in ["claude-cli", "acp", "mcp", "http", "ollama", "sim"] {
                this.set_provider_enabled(id, false, cx);
            }
        });
    });
    assert_compare_inert(&ws, cx, "nothing to compare against");
}

#[test]
fn compare_item_disabled_when_nothing_to_send() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert_compare_inert(&ws, cx, "empty transcript and empty draft — nothing to send");
}

/// …but an empty transcript with a draft DOES offer the item — the draft
/// is the prompt.
#[test]
fn compare_item_enabled_by_draft_alone() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("draft only", window, cx));
        });
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        let item = compare_menu_item(window);
        assert!(item.disabled() != Some(true), "a draft alone enables compare");
    });
}
