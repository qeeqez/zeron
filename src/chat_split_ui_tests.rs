//! Headless tests for split-chat's persisted and UI surface: both halves
//! save through the normal path, checkpoints/feedback partition with the
//! messages, an ephemeral source splits ephemeral, and the message ⋯
//! menu plus the ⋯ "Split chat…" dialog drive `split_chat` end to end.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-splitui-test-{}", std::process::id()));
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
            pinned: false,
            usage: None,
            attachments: vec![],
            alternatives: vec![],
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
fn split_persists_both_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 2, window, cx));
    });
    let dir = app.read(|cx| ws.read(cx).project.chats_dir());
    let mut next_id = 100;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    let src = loaded.iter().find(|c| c.title == "Fix bug").expect("source should persist");
    assert_eq!(src.messages.len(), 2, "the persisted source is truncated");
    let split = loaded.iter().find(|c| c.title == "Fix bug (split)").expect("split should persist");
    assert_eq!(split.messages.len(), 2, "the persisted split holds the suffix");
}

#[test]
fn split_prunes_source_checkpoints() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    ws.update(cx, |this, _| {
        let chat = &mut this.chats[0];
        chat.checkpoints = vec![
            crate::checkpoints::TurnCheckpoint {
                ix: 0,
                at: chat.messages[0].at,
                checkpoint: crate::checkpoints::Checkpoint::Copy("/tmp/cp0".into()),
            },
            crate::checkpoints::TurnCheckpoint {
                ix: 2,
                at: chat.messages[2].at,
                checkpoint: crate::checkpoints::Checkpoint::Copy("/tmp/cp2".into()),
            },
        ];
    });
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 2, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats[0].checkpoints.len(), 1, "the source keeps checkpoints before the split");
        assert_eq!(ws.chats[0].checkpoints[0].ix, 0);
        assert!(ws.chats[1].checkpoints.is_empty(), "the split starts with no checkpoints");
    });
}

#[test]
fn split_feedback_follows_its_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    ws.update(cx, |this, _| {
        let chat = &mut this.chats[0];
        chat.feedback = vec![
            crate::feedback::FeedbackNote { at: chat.messages[1].at, note: "kept".into() },
            crate::feedback::FeedbackNote { at: chat.messages[3].at, note: "moved".into() },
        ];
    });
    let src_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 2, window, cx));
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats[0].feedback.len(), 1);
        assert_eq!(ws.chats[0].feedback[0].note, "kept");
        assert_eq!(ws.chats[1].feedback.len(), 1);
        assert_eq!(ws.chats[1].feedback[0].note, "moved");
    });
}

#[test]
fn ephemeral_source_splits_ephemeral() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.new_temp_chat(cx)));
    seed_transcript(&ws, cx);
    // The temp chat is the active one — `new_temp_chat` appends it.
    let src_ix = cx.update(|_, cx| ws.read(cx).active);
    let src_id = cx.update(|_, cx| ws.read(cx).chats[src_ix].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.split_chat(src_id, 2, window, cx));
    });
    let dir = app.read(|cx| {
        let ws = ws.read(cx);
        assert!(ws.chats[src_ix].ephemeral, "source stays ephemeral");
        assert!(ws.chats[src_ix + 1].ephemeral, "the split is ephemeral too");
        ws.project.chats_dir()
    });
    let mut next_id = 100;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    assert!(loaded.iter().all(|c| c.title != "Fix bug (split)"), "an ephemeral split never reaches disk");
}

/// Right-clicking a non-first message offers "Split chat here"; choosing
/// it moves the tail into a new chat after the source.
#[test]
fn context_menu_split_here_moves_tail() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        window.right_click(("msg", 2usize), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Split chat here"))
            .unwrap_or_else(|| panic!("menu should offer Split chat here"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
    });
    app.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "Split chat here opens a new chat");
        assert_eq!(ws.active, 1, "the split is selected");
        assert_eq!(ws.chats[1].title.as_ref(), "Fix bug (split)");
        assert_eq!(ws.chats[1].messages.len(), 2, "the split holds the tail");
        assert_eq!(ws.chats[0].messages.len(), 2, "the source keeps the prefix");
    });
}

/// The first message's menu can't split — nothing would stay behind.
#[test]
fn context_menu_first_message_has_no_split() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        assert!(snapshots(window).iter().all(|s| s.label() != Some("Split chat here")), "the first message must not offer Split chat here");
    });
}

/// The ⋯ menu's "Split chat…" opens the dialog; typing a 1-based message
/// number and confirming splits there.
#[test]
fn chat_menu_split_dialog_splits_at_number() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_transcript(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Split chat…"))
            .unwrap_or_else(|| panic!("chat menu should offer Split chat…"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_some(), "Split chat… should open the dialog");
        ws.read(cx).split_input.clone().update(cx, |s, cx| s.set_value("3", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "dialog OK should split the chat");
        assert_eq!(ws.chats[0].messages.len(), 2, "message 3 starts the new chat");
        assert_eq!(text_of(&ws.chats[1].messages[0]), "u2");
        assert_eq!(ws.active, 1, "the split is selected");
        assert!(window.try_find("dialog").is_none(), "dialog should close on OK");
    });
}
