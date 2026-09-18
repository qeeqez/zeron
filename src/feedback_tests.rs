//! Tests for message feedback: thumbs up/down toggling and switching, the
//! thumbs-down "what went wrong" note (commit, cancel, removal), save/load
//! persistence, and the headless footer path — hover reveals the buttons,
//! clicking marks the message, Enter commits the note.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::feedback::FeedbackNote;
use crate::model::{Chat, ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-feedback-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process.
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

/// Push a message without starting a reply.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
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

/// The active chat's messages and notes.
fn chat_of(ws: &Workspace) -> &Chat {
    &ws.chats[ws.active]
}

#[test]
fn rating_sets_toggles_and_switches() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "reply");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.rate_message(0, true, window, cx);
            assert_eq!(chat_of(this).messages[0].rating, Some(true));
            // Re-clicking the same thumb clears the rating.
            this.rate_message(0, true, window, cx);
            assert_eq!(chat_of(this).messages[0].rating, None);
            // The other thumb sets, then switches back.
            this.rate_message(0, false, window, cx);
            assert_eq!(chat_of(this).messages[0].rating, Some(false));
            this.rate_message(0, true, window, cx);
            assert_eq!(chat_of(this).messages[0].rating, Some(true));
        });
    });
    // Thumbs-down opened the note editor; switching to up closed it.
    ws.read_with(cx, |ws, _| assert!(ws.feedback.editing.is_none()));
}

#[test]
fn user_messages_cant_be_rated() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "question");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.rate_message(0, true, window, cx));
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(chat_of(ws).messages[0].rating, None);
        assert!(ws.feedback.editing.is_none());
    });
}

#[test]
fn thumbs_down_note_commits_and_empty_commit_removes() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "reply");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.rate_message(0, false, window, cx);
            assert!(this.feedback.editing.is_some(), "thumbs-down opens the note editor");
            this.feedback.input.update(cx, |s, cx| s.set_value("wrong file", window, cx));
            this.commit_feedback(window, cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        let chat = chat_of(ws);
        assert_eq!(chat.feedback.len(), 1);
        assert_eq!(chat.feedback[0].note, "wrong file");
        assert_eq!(chat.feedback[0].at, chat.messages[0].at);
        assert!(ws.feedback.editing.is_none());
    });
    // Reopen via the note affordance, clear the text, commit → note removed.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.edit_feedback_note(0, window, cx);
            assert_eq!(this.feedback.input.read(cx).value(), "wrong file", "editor reseeds the saved note");
            this.feedback.input.update(cx, |s, cx| s.set_value("", window, cx));
            this.commit_feedback(window, cx);
        });
    });
    ws.read_with(cx, |ws, _| assert!(chat_of(ws).feedback.is_empty()));
}

#[test]
fn cancel_keeps_the_note_unwritten() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "reply");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.rate_message(0, false, window, cx);
            this.feedback.input.update(cx, |s, cx| s.set_value("draft", window, cx));
            this.cancel_feedback(window, cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        assert!(chat_of(ws).feedback.is_empty(), "cancel never stores the draft");
        assert!(ws.feedback.editing.is_none());
        // The thumbs-down rating itself stays.
        assert_eq!(chat_of(ws).messages[0].rating, Some(false));
    });
}

#[test]
fn switching_to_thumbs_up_drops_the_note() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "reply");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.rate_message(0, false, window, cx);
            this.feedback.input.update(cx, |s, cx| s.set_value("bad", window, cx));
            this.commit_feedback(window, cx);
            this.rate_message(0, true, window, cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(chat_of(ws).messages[0].rating, Some(true));
        assert!(chat_of(ws).feedback.is_empty(), "a thumbs-up can't carry a note");
    });
}

#[test]
fn rating_and_note_survive_save_load() {
    let dir = std::env::temp_dir().join(format!("rixlcode-feedback-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut chat = Chat::new(0, "rated");
    let at = std::time::SystemTime::now();
    std::rc::Rc::make_mut(&mut chat.messages).push(ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Text("answer".into()),
        rating: Some(false),
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at,
    });
    chat.feedback.push(FeedbackNote { at, note: "hallucinated the API".into() });
    crate::persist::save_chats(&dir, &[chat]);
    let mut next_id = 0;
    let mut loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    crate::persist::hydrate_all(&mut loaded, &dir);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].messages[0].rating, Some(false));
    assert_eq!(loaded[0].feedback.as_slice(), [FeedbackNote { at, note: "hallucinated the API".into() }].as_slice());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_drops_notes_whose_message_is_gone() {
    // A note anchored to a message truncated away (edit-resend, /clear)
    // must not persist — the anchor check runs at save time.
    let dir = std::env::temp_dir().join(format!("rixlcode-feedback-orphan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut chat = Chat::new(0, "orphan");
    chat.feedback.push(FeedbackNote { at: std::time::SystemTime::now(), note: "stale".into() });
    crate::persist::save_chats(&dir, &[chat]);
    let mut next_id = 0;
    let mut loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    crate::persist::hydrate_all(&mut loaded, &dir);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].feedback.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The hover footer's thumbs buttons mark the message; thumbs-down mounts
/// the note editor and Enter attaches the note under the footer.
#[test]
fn footer_buttons_mark_the_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::Assistant, "reply");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        for id in ["up", "down"] {
            assert!(window.find((id, 0usize)).visible(), "{id} reveals on hover");
        }
        window.click(("up", 0usize), cx);
        assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(true));
        window.click(("down", 0usize), cx);
        assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(false));
        window.draw(cx).clear(cx);
        assert!(window.find(("feedback-edit", 0usize)).visible(), "thumbs-down mounts the note editor");
    });
    cx.update(|window, cx| {
        // The deferred focus lands between updates; make it explicit so the
        // typed text can't leak into another input.
        ws.update(cx, |this, cx| {
            this.feedback.input.update(cx, |s, cx| s.focus(window, cx));
        });
        window.input("it made stuff up", cx);
        window.press("enter", cx);
    });
    // The PressEnter subscription commits on the next effect flush.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("feedback-edit", 0usize)).is_none(), "Enter commits and closes the editor");
        assert!(window.find(("feedback-note", 0usize)).visible(), "the note shows under the footer");
    });
    ws.read_with(cx, |ws, _| {
        let chat = chat_of(ws);
        assert_eq!(chat.feedback.len(), 1);
        assert_eq!(chat.feedback[0].note, "it made stuff up");
    });
}
