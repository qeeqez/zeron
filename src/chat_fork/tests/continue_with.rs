//! "Continue with…" handoff tests: the ⋯ menu's provider submenu and the
//! `continue_chat_with` fork it drives — transcript copy, provider rebind,
//! thread-id reset, and the states that disable the item.

use gpui_kit::base::test_support::{ElementSnapshot, snapshots};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, Window};

use super::{mount, seed_transcript};
use crate::model::MessageKind;
use crate::workspace::Workspace;

/// The ⋯ menu on the chat titlebar — opened by clicking the header button.
fn open_chat_menu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
    });
}

fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window).iter().filter_map(|s| s.label().map(|l| l.to_string())).collect()
}

/// The "Continue with" menu row's snapshot — panics when it isn't offered.
fn continue_with_item(window: &Window) -> ElementSnapshot {
    snapshots(window)
        .iter()
        .find(|s| s.label() == Some("Continue with"))
        .unwrap_or_else(|| panic!("chat menu should offer Continue with"))
        .clone()
}

/// GPUI exposes no aria-disabled flag, so the off state is observable only
/// as inertness: hovering the row opens no submenu and clicking it forks
/// nothing.
fn assert_continue_with_inert(ws: &Entity<Workspace>, cx: &mut VisualTestContext, because: &str) {
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = continue_with_item(window);
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").hover(id.clone(), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("submenu").is_none(), "{because}");
        window.within("popup-menu").click(id, cx);
    });
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).chats.len(), 1, "{because}");
    });
}

#[test]
fn continue_with_forks_transcript_onto_new_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let chat = &mut this.chats[0];
            chat.provider = "codex-cli".into();
            chat.model = "gpt-5-codex".into();
            chat.thread_id = "thread-123".into();
        });
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.continue_chat_with(0, "claude-cli", window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "continue adds a chat");
        assert_eq!(ws.active, 1, "the continuation is selected");
        let fork = &ws.chats[1];
        assert_eq!(fork.title.as_ref(), "Fix bug · Claude");
        assert_eq!(fork.messages.len(), 4, "the whole transcript carries over");
        assert!(matches!(&fork.messages[0].kind, MessageKind::Text(t) if t.as_ref() == "u1"));
        assert_eq!(fork.provider, "claude-cli", "rebound to the picked provider");
        assert_eq!(fork.model, "sonnet", "lands on the new provider's first model");
        assert!(fork.thread_id.is_empty(), "the old backend thread must not leak across providers");
        let src = &ws.chats[0];
        assert_eq!(src.provider, "codex-cli", "original keeps its binding");
        assert_eq!(src.thread_id, "thread-123", "original keeps its thread");
        assert_eq!(src.messages.len(), 4, "original keeps its transcript");
        // The workspace selection follows the fork — sends go to Claude.
        assert_eq!(ws.selected_provider, "claude-cli");
        assert_eq!(ws.backend.name(), "claude-cli");
    });
}

#[test]
fn continue_with_same_provider_is_a_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| this.chats[0].provider = "codex-cli".into());
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.continue_chat_with(0, "codex-cli", window, cx));
    });
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats.len(), 1, "continuing on the same provider forks nothing");
    });
}

#[test]
fn continue_with_disabled_provider_is_a_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.set_provider_enabled("claude-cli", false, cx));
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.continue_chat_with(0, "claude-cli", window, cx));
    });
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats.len(), 1, "a disabled instance can't take the handoff");
    });
}

/// The ⋯ menu's "Continue with" lists every enabled instance except the
/// chat's own; picking one forks the transcript onto it.
#[test]
fn chat_menu_continue_with_hands_off_to_picked_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| this.chats[0].provider = "codex-cli".into());
    });
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = continue_with_item(window);
        assert!(item.disabled() != Some(true), "enabled with other providers configured");
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").hover(id, cx);
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        for name in ["Claude", "ACP", "HTTP", "Ollama", "Sim"] {
            assert!(labels.iter().any(|l| l == name), "submenu offers {name}: {labels:?}");
        }
        assert!(!labels.iter().any(|l| l == "Codex"), "the chat's own provider is excluded: {labels:?}");
        let claude = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Claude"))
            .unwrap_or_else(|| panic!("submenu should offer Claude"))
            .clone();
        window.within("submenu").click(claude.path().last().unwrap().clone(), cx);
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "the pick forks the chat");
        assert_eq!(ws.chats[1].provider, "claude-cli");
        assert_eq!(ws.chats[1].title.as_ref(), "Fix bug · Claude");
        assert_eq!(ws.chats[1].messages.len(), 4);
    });
}

#[test]
fn continue_with_disabled_while_running() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| this.chats[0].running = true);
    });
    assert_continue_with_inert(&ws, cx, "no handoff mid-turn");
}

#[test]
fn continue_with_disabled_with_a_single_provider() {
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
    assert_continue_with_inert(&ws, cx, "nothing to continue with");
}

#[test]
fn continue_with_disabled_on_an_empty_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert_continue_with_inert(&ws, cx, "nothing to hand off yet");
}
