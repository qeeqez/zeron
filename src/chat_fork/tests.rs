//! Headless tests for fork-here: `fork_chat` branches the transcript into a
//! new chat without touching the original, and the message context menu's
//! "Fork from here" drives it end to end.

mod compare;
mod continue_with;
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-fork-test-{}", std::process::id()));
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
            alternatives: vec![],
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
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

#[test]
fn fork_copies_messages_up_to_point() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.fork_chat(0, Some(1), window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "fork adds a chat");
        let fork = &ws.chats[1];
        assert_eq!(fork.title.as_ref(), "Fix bug · fork");
        assert_eq!(fork.messages.len(), 2, "fork holds messages up to and including ix 1");
        assert!(matches!(&fork.messages[0].kind, MessageKind::Text(t) if t.as_ref() == "u1"));
        assert!(matches!(&fork.messages[1].kind, MessageKind::Text(t) if t.as_ref() == "a1"));
    });
}

#[test]
fn fork_leaves_original_untouched() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.fork_chat(0, Some(0), window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        let src = &ws.chats[0];
        assert_eq!(src.id, src_id);
        assert_eq!(src.title.as_ref(), "Fix bug", "original title unchanged");
        assert_eq!(src.messages.len(), 4, "original keeps its full transcript");
        assert_ne!(src.id, ws.chats[1].id, "fork gets a fresh id");
    });
}

#[test]
fn fork_is_selected_and_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.fork_chat(0, Some(2), window, cx));
    });
    let dir = app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.active, 1, "the fork is selected");
        assert_eq!(ws.chats[1].messages.len(), 3);
        ws.project.chats_dir()
    });
    let mut next_id = 100;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    let fork = loaded.iter().find(|c| c.title == "Fix bug · fork").expect("fork should persist");
    assert_eq!(fork.messages.len(), 3, "persisted fork holds the truncated transcript");
    assert!(loaded.iter().any(|c| c.title == "Fix bug" && c.messages.len() == 4), "original persists intact");
}

#[test]
fn fork_past_end_is_a_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.fork_chat(0, Some(99), window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 1, "no fork past the end of the transcript");
        assert_eq!(ws.active, 0);
    });
}

#[test]
fn fork_none_copies_whole_transcript() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.fork_chat(0, None, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2);
        assert_eq!(ws.chats[1].messages.len(), 4, "None forks at the end — the whole transcript");
        assert_eq!(ws.chats[1].title.as_ref(), "Fix bug · fork");
    });
}

/// Right-clicking a message offers "Fork from here"; choosing it opens a
/// new chat holding the transcript through that message.
#[test]
fn context_menu_fork_here_opens_new_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        window.right_click(("msg", 1usize), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        let fork = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Fork from here"))
            .unwrap_or_else(|| panic!("menu should offer Fork from here"))
            .clone();
        let id = fork.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "Fork from here opens a new chat");
        assert_eq!(ws.active, 1, "the fork is selected");
        assert_eq!(ws.chats[1].title.as_ref(), "Fix bug · fork");
        assert_eq!(ws.chats[1].messages.len(), 2, "fork holds messages through the clicked one");
        assert_eq!(ws.chats[0].messages.len(), 4, "original untouched");
    });
}

/// The fork keeps the source's provider/model binding but never its
/// backend thread id — a backend thread can't be partially rewound, so
/// the fork's first send starts a fresh thread.
#[test]
fn fork_keeps_binding_and_starts_fresh_thread() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    ws.update(cx, |this, _| {
        let src = &mut this.chats[this.active];
        src.provider = "openai".into();
        src.model = "gpt-5".into();
        src.thread_id = "thread-123".into();
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.fork_chat(0, Some(1), window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        let fork = &ws.chats[1];
        assert_eq!(fork.provider.as_str(), "openai", "fork keeps the provider binding");
        assert_eq!(fork.model.as_str(), "gpt-5", "fork keeps the model binding");
        assert!(fork.thread_id.is_empty(), "fork starts a fresh backend thread");
    });
}

/// "Fork from here" is hidden on the last message — forking the whole
/// transcript duplicates the chat, which "Fork chat" already does.
#[test]
fn context_menu_fork_hidden_on_last_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        window.right_click(("msg", 3usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        assert!(
            snapshots(window).iter().all(|s| s.label() != Some("Fork from here")),
            "the last message's menu should not offer Fork from here"
        );
    });
}

/// While a turn runs the item stays listed but inert — GPUI exposes no
/// aria-disabled flag on menu items, so the disabled state is observable
/// only as a click that forks nothing.
#[test]
fn context_menu_fork_disabled_while_running() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    ws.update(cx, |this, _| this.chats[this.active].running = true);
    cx.update(|window, cx| {
        window.right_click(("msg", 1usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        let fork = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Fork from here"))
            .unwrap_or_else(|| panic!("menu should still list Fork from here"))
            .clone();
        window.within("popup-menu").click(fork.path().last().unwrap().clone(), cx);
    });
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats.len(), 1, "a disabled item can't fork");
    });
}
