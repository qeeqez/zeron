//! Headless tests for split-chat: `split_chat` moves the transcript tail
//! into a new chat inserted right after the source, which keeps the
//! prefix. The UI surface (message menu, ⋯ dialog) lives in
//! `chat_split_ui_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-split-test-{}", std::process::id()));
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

/// Append a message to the active chat without starting a reply.
fn seed(ws: &Entity<Workspace>, role: Role, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role,
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

/// Seed a two-turn transcript: u1, a1, u2, a2.
fn seed_transcript(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    seed(ws, Role::User, "u1", cx);
    seed(ws, Role::Assistant, "a1", cx);
    seed(ws, Role::User, "u2", cx);
    seed(ws, Role::Assistant, "a2", cx);
    ws.update(cx, |this, _| this.chats[this.active].title = "Fix bug".into());
}

fn text_of(msg: &ChatMessage) -> &str {
    match &msg.kind {
        MessageKind::Text(t) => t.as_ref(),
        _ => panic!("expected a text message"),
    }
}

#[test]
fn split_moves_suffix_into_new_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    let src_id = cx.update(|window, cx| {
        let id = ws.read(cx).chats[0].id;
        ws.update(cx, |this, cx| this.split_chat(id, 2, window, cx));
        id
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "split adds a chat");
        let src = &ws.chats[0];
        assert_eq!(src.id, src_id, "source keeps its id");
        assert_eq!(src.title.as_ref(), "Fix bug", "source title unchanged");
        assert_eq!(src.messages.len(), 2, "source keeps the prefix");
        assert_eq!(text_of(&src.messages[0]), "u1");
        assert_eq!(text_of(&src.messages[1]), "a1");
        let split = &ws.chats[1];
        assert_ne!(split.id, src_id, "split gets a fresh id");
        assert_eq!(split.title.as_ref(), "Fix bug (split)");
        assert_eq!(split.messages.len(), 2, "split holds the suffix");
        assert_eq!(text_of(&split.messages[0]), "u2");
        assert_eq!(text_of(&split.messages[1]), "a2");
        assert!(split.thread_id.is_empty(), "split starts a fresh backend thread");
        assert_eq!(ws.active, 1, "the split becomes active");
    });
}

#[test]
fn split_inserts_after_source() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    // A second chat after the source — the split must land between them.
    cx.update(|_, cx| ws.update(cx, |this, cx| this.new_chat(cx)));
    let (src_id, tail_id) = cx.update(|_, cx| {
        let ws = ws.read(cx);
        (ws.chats[0].id, ws.chats[1].id)
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 1, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 3);
        assert_eq!(ws.chats[0].id, src_id);
        assert_eq!(ws.chats[1].title.as_ref(), "Fix bug (split)", "split sits right after the source");
        assert_eq!(ws.chats[2].id, tail_id, "the later chat shifts down");
        assert_eq!(ws.active, 1);
    });
}

#[test]
fn split_inherits_thread_stamps() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    ws.update(cx, |this, _| {
        let chat = &mut this.chats[0];
        chat.provider = "codex".into();
        chat.model = "gpt-5".into();
        chat.access = Some(crate::backend::AccessMode::FullAccess);
        chat.effort = Some("high".into());
        chat.instructions = Some("be terse".into());
        chat.color = Some(crate::model::ChatColor::Blue);
        chat.folder = "Work".into();
        chat.thread_id = "thread-123".into();
    });
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 2, window, cx));
    });
    app.read(|cx| {
        let split = &ws.read(cx).chats[1];
        assert_eq!(split.provider, "codex");
        assert_eq!(split.model, "gpt-5");
        assert_eq!(split.access, Some(crate::backend::AccessMode::FullAccess));
        assert_eq!(split.effort.as_deref(), Some("high"));
        assert_eq!(split.instructions.as_deref(), Some("be terse"));
        assert_eq!(split.color, Some(crate::model::ChatColor::Blue));
        assert_eq!(split.folder, "Work");
        assert!(split.thread_id.is_empty(), "the backend thread is not inherited");
    });
}

#[test]
fn split_at_zero_is_a_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 0, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 1, "splitting at the first message leaves nothing behind");
        assert_eq!(ws.chats[0].messages.len(), 4);
        assert_eq!(ws.active, 0);
    });
}

#[test]
fn split_empty_chat_is_a_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let empty_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(empty_id, 1, window, cx));
    });
    app.read(|cx| assert_eq!(ws.read(cx).chats.len(), 1, "an empty chat can't split"));
}

#[test]
fn split_past_end_is_a_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 4, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 1, "splitting past the end is a no-op");
        assert_eq!(ws.chats[0].messages.len(), 4);
    });
}

#[test]
fn split_running_chat_is_blocked() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    ws.update(cx, |this, _| this.chats[0].running = true);
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 2, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 1, "a running chat can't split");
        assert_eq!(ws.chats[0].messages.len(), 4);
    });
}
