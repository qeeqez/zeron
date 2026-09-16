//! Headless tests for per-chat custom instructions: set/clear through
//! `set_chat_instructions`, the `TurnContext` merge order (global →
//! project file → per-chat), persistence round-trips, ephemeral chats
//! staying off disk, the ⋯ menu item, the titlebar badge, and the dialog.
//! Same mount harness as `chat_color_tests.rs` — duplicated because
//! sibling test files can't share private helpers.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::model::Chat;
use crate::workspace::Workspace;

/// A backend that records each turn's `TurnContext` instead of spawning —
/// the instructions assertion without a real subprocess.
struct RecordingBackend {
    ctxs: std::sync::Arc<parking_lot::Mutex<Vec<TurnContext>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, ctx: &TurnContext) -> ReplyStream {
        self.ctxs.lock().push(ctx.clone());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// A fresh temp dir (HOME and project roots both live under it).
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-chatinstr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: crate::project::Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let home = temp_dir(&format!("{name}-home"));
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &home) };
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

#[test]
fn set_and_clear_chat_instructions() {
    let root = temp_dir("set");
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "set", crate::project::Project::open(&root));
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "always answer in haiku", cx));
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).chats[0].instructions.as_deref(), Some("always answer in haiku"));
    });
    // Empty input clears the override back to None.
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "   ", cx));
    cx.update(|_, cx| assert_eq!(ws.read(cx).chats[0].instructions, None, "empty input clears to None"));
}

#[test]
fn chat_instructions_merge_after_global_and_project() {
    let root = temp_dir("ctx");
    std::fs::write(root.join("AGENTS.md"), "project rules").unwrap();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "ctx", crate::project::Project::open(&root));
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.instructions = "global rules".to_string();
            let id = this.chats[this.active].id;
            this.set_chat_instructions(id, "chat rules", cx);
            this.backend = std::sync::Arc::new(RecordingBackend { ctxs: ctxs.clone() });
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    let ctxs = ctxs.lock();
    assert_eq!(ctxs.len(), 1);
    assert_eq!(ctxs[0].instructions.as_deref(), Some("global rules\n\nproject rules\n\nchat rules"));
}

#[test]
fn cleared_chat_instructions_leave_turn_context() {
    let root = temp_dir("ctx-clear");
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "ctx-clear", crate::project::Project::open(&root));
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            let id = this.chats[this.active].id;
            this.set_chat_instructions(id, "chat rules", cx);
            this.set_chat_instructions(id, "", cx);
            this.backend = std::sync::Arc::new(RecordingBackend { ctxs: ctxs.clone() });
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    assert_eq!(ctxs.lock()[0].instructions, None, "a cleared override must not reach the turn");
}

#[test]
fn instructions_survive_save_and_reload() {
    let root = temp_dir("persist");
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "persist", crate::project::Project::open(&root));
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "be terse", cx));
    cx.update(|_, cx| {
        let dir = ws.read(cx).project.chats_dir();
        let mut next_id = 0;
        let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].instructions.as_deref(), Some("be terse"), "instructions must round-trip through the chat file");
    });
    // Clearing persists too — the reloaded chat has no override.
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "", cx));
    cx.update(|_, cx| {
        let dir = ws.read(cx).project.chats_dir();
        let mut next_id = 0;
        let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
        assert_eq!(loaded[0].instructions, None, "a cleared override must not resurrect");
    });
}

#[test]
fn temp_chat_instructions_never_reach_disk() {
    let dir = temp_dir("temp");
    let mut temp = Chat::new(1, "temp");
    temp.ephemeral = true;
    temp.instructions = Some("secret rules".to_string());
    crate::persist::save_chats(&dir, &[Chat::new(0, "normal"), temp]);
    assert!(dir.join("0.json").exists(), "the normal chat persists");
    assert!(!dir.join("1.json").exists(), "a temporary chat's instructions still write no file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chat_menu_offers_custom_instructions() {
    let root = temp_dir("menu");
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app, "menu", crate::project::Project::open(&root));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(
            snapshots(window)
                .iter()
                .any(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some("Custom instructions…")),
            "chat menu should offer Custom instructions…"
        );
    });
}

#[test]
fn titlebar_shows_instructions_badge() {
    let root = temp_dir("badge");
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "badge", crate::project::Project::open(&root));
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("instructions-badge").is_none(), "chats without an override show no badge");
    });
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "be terse", cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("instructions-badge").visible(), "the titlebar shows the instructions badge");
    });
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "", cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("instructions-badge").is_none(), "clearing the override removes the badge");
    });
}

#[test]
fn dialog_prefills_and_saves() {
    let root = temp_dir("dialog");
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "dialog", crate::project::Project::open(&root));
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_instructions(chat_id, "existing rules", cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| this.open_chat_instructions(chat_id, window, cx));
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_some(), "the instructions dialog should open");
        assert_eq!(
            ws.read(cx).chat_instructions_input.read(cx).value().as_ref(),
            "existing rules",
            "the dialog seeds the chat's current text"
        );
        ws.read(cx).chat_instructions_input.clone().update(cx, |s, cx| s.set_value("new rules", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].instructions.as_deref(), Some("new rules"), "OK should save the edit");
        assert!(window.try_find("dialog").is_none(), "dialog should close on OK");
    });
}
