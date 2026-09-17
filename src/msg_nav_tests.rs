//! Tests for `msg_nav` — keyboard navigation through the transcript. Esc from
//! an idle composer focuses the newest message; j/k (and ↑/↓) step the focus
//! cursor, gg/G jump to the ends, Enter edits a user message or copies any
//! other, and Esc or a transcript click hands focus back to the composer.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, Entity, Focusable, TestAppContext, VisualTestContext, Window};

use crate::composer_testutil::open_workspace;
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Append a text message to the active chat and grow the scroller.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, s: &str) {
    ws.update(cx, |this, cx| {
        let chat = &mut this.chats[this.active];
        std::rc::Rc::make_mut(&mut chat.messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(s.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

fn nav_ix(ws: &Entity<Workspace>, cx: &App) -> Option<usize> {
    ws.read_with(cx, |ws, _| ws.nav.map(|n| n.ix))
}

fn composer_focused(ws: &Entity<Workspace>, window: &Window, cx: &mut App) -> bool {
    ws.update(cx, |ws, cx| ws.composer.read(cx).focus_handle(cx).is_focused(window))
}

/// Esc from the idle composer enters navigation on the newest message; j/k
/// step the cursor and each focused row carries the highlight.
#[test]
fn esc_enters_nav_and_jk_step_the_cursor() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "second");
    push(&ws, cx, Role::User, "third");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        assert!(composer_focused(&ws, window, cx), "workspace opens with the composer focused");

        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(2), "esc focuses the newest message");
        assert!(window.find("msg-nav-focus").visible(), "focused row shows the highlight");
        assert!(!composer_focused(&ws, window, cx), "navigation takes focus off the composer");

        window.press("k", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1), "k moves to the previous message");

        window.press("k", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(0));
        window.press("k", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(0), "k clamps at the first message");

        window.press("j", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1), "j moves to the next message");
        window.press("down", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(2), "↓ mirrors j");
        window.press("j", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(2), "j clamps at the last message");
    });
}

/// gg jumps to the first message, G to the last; a lone g only arms the
/// double-tap prefix.
#[test]
fn gg_and_g_jump_to_the_ends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    for i in 0..5 {
        push(&ws, cx, Role::Assistant, &format!("msg {i}"));
    }
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(4));

        window.press("g", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(4), "a lone g only arms the prefix");
        window.press("g", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(0), "gg jumps to the first message");

        window.press("shift-g", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(4), "G jumps to the last message");
    });
}

/// Enter on a focused user message opens the inline editor; on an assistant
/// message it copies the text to the clipboard.
#[test]
fn enter_edits_user_messages_and_copies_assistant_ones() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push(&ws, cx, Role::User, "edit me");
    push(&ws, cx, Role::Assistant, "copy me");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1));

        // Enter on the assistant message copies it and keeps the cursor.
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1), "copying keeps the cursor in place");
    });
    assert_eq!(
        app.read_from_clipboard().and_then(|item| item.text()),
        Some("copy me".to_string()),
        "enter on an assistant message copies its text"
    );

    cx.update(|window, cx| {
        // Esc exits to the composer; a second Esc re-enters at the newest
        // message, k steps to the user message, Enter opens its editor.
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), None);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1));
        window.press("k", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(0));
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("msg-edit", 0usize)).visible(), "enter on a user message opens its editor");
        assert!(ws.read(cx).editing.is_some());
    });
}

/// Esc exits navigation and refocuses the composer; a transcript click does
/// the same.
#[test]
fn esc_and_click_return_focus_to_the_composer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push(&ws, cx, Role::User, "one");
    push(&ws, cx, Role::Assistant, "two");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1));

        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), None, "esc clears the cursor");
        assert!(composer_focused(&ws, window, cx), "esc returns focus to the composer");
        assert!(window.try_find("msg-nav-focus").is_none(), "the highlight is gone");

        // Re-enter, then click the transcript — focus returns to the composer.
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), Some(1));
        window.click(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        assert_eq!(nav_ix(&ws, cx), None, "clicking clears the cursor");
        assert!(composer_focused(&ws, window, cx), "clicking returns focus to the composer");
    });
}

/// With the composer focused, j/k/g type into it instead of navigating.
#[test]
fn nav_keys_type_into_the_focused_composer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push(&ws, cx, Role::Assistant, "a reply");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        assert!(composer_focused(&ws, window, cx));
        window.input("jkg", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).composer.read(cx).value().to_string(), "jkg", "letters reach the composer");
        assert_eq!(nav_ix(&ws, cx), None, "typing never arms navigation");
    });
}

/// Under a chat-search filter the cursor steps through matching messages
/// only — the same visible list the scroller renders.
#[test]
fn nav_steps_over_filtered_matches_only() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push(&ws, cx, Role::User, "hit one");
    push(&ws, cx, Role::Assistant, "skip me");
    push(&ws, cx, Role::Assistant, "hit two");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.chat_search_open = true;
            this.chat_search.update(cx, |s, cx| s.set_value("hit", window, cx));
            let count = this.filtered_count(cx);
            this.scroller.update(cx, |s, cx| s.reset(count, cx));
        });
        let handle = ws.read(cx).nav_focus.clone();
        window.focus(&handle, cx);
        window.draw(cx).clear(cx);

        ws.update(cx, |this, cx| this.nav_move(false, window, cx));
        assert_eq!(nav_ix(&ws, cx), Some(2), "entry lands on the last match");
        ws.update(cx, |this, cx| this.nav_move(true, window, cx));
        assert_eq!(nav_ix(&ws, cx), Some(0), "k skips filtered-out rows");
        ws.update(cx, |this, cx| this.nav_move(true, window, cx));
        assert_eq!(nav_ix(&ws, cx), Some(0), "k clamps at the first match");
        ws.update(cx, |this, cx| this.nav_move(false, window, cx));
        assert_eq!(nav_ix(&ws, cx), Some(2), "j steps to the next match");
    });
}
