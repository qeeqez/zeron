//! Tests for the Bookmarks panel: the sidebar row and Cmd-Shift-B toggle it,
//! the panel lists starred messages across every loaded chat grouped by
//! title, a row click opens its chat and scrolls to the message, the ×
//! unstars in place, "Clear all" empties the list, and the open flag
//! persists. Same harness as `bookmark_tests.rs`.

use std::rc::Rc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, KeyBinding, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-bmpanel-test-{}", std::process::id()));
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

/// Push a text message without starting a turn, growing the scroller.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

/// Star message `ix` in the active chat.
fn star(ws: &Entity<Workspace>, cx: &mut VisualTestContext, ix: usize) {
    ws.update(cx, |this, cx| this.toggle_bookmark(ix, cx));
}

#[test]
fn panel_lists_bookmarks_across_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "first chat's starred question");
    star(&ws, cx, 0);
    let first_id = ws.read_with(cx, |ws, _| ws.chats[0].id);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[this.active].title = "Second chat".into();
        });
    });
    push(&ws, cx, Role::Assistant, "second chat's starred answer");
    star(&ws, cx, 0);
    let second_id = ws.read_with(cx, |ws, _| ws.chats[1].id);

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("sidebar-bookmarks", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("bookmarks-panel").visible(), "the sidebar row opens the panel");
        assert_eq!(window.find(format!("bm-group-{first_id}")).label(), Some("New chat"));
        assert_eq!(window.find(format!("bm-group-{second_id}")).label(), Some("Second chat"));
        assert_eq!(window.find(format!("bm-row-{first_id}-0")).label(), Some("first chat's starred question"));
        assert_eq!(window.find(format!("bm-row-{second_id}-0")).label(), Some("second chat's starred answer"));
        assert_eq!(window.find("sidebar-bookmarks-count").label(), Some("2"));
    });
}

/// A row click switches to the bookmark's chat and scrolls the transcript
/// to the message — the same jump the ⋯ submenu performs.
#[test]
fn row_click_opens_chat_and_jumps() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "the pinned question");
    for i in 0..40 {
        push(&ws, cx, Role::Assistant, &format!("filler reply {i}"));
    }
    star(&ws, cx, 0);
    let first_id = ws.read_with(cx, |ws, _| ws.chats[0].id);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.new_chat(cx));
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.bookmarks_panel.open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).active, 1, "the second chat is active");
        window.click(format!("bm-row-{first_id}-0"), cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).active, 0, "the click switches to the bookmark's chat");
        assert!(window.find(("msg", 0usize)).visible(), "the transcript scrolls to the message");
    });
}

/// The row's × unstars the message in its own chat — the row disappears
/// and the flag clears, without switching chats.
#[test]
fn unmark_removes_the_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "keep me starred");
    push(&ws, cx, Role::Assistant, "drop me");
    star(&ws, cx, 0);
    star(&ws, cx, 1);
    let chat_id = ws.read_with(cx, |ws, _| ws.chats[0].id);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.bookmarks_panel.open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.click(format!("bm-unmark-{chat_id}-1"), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(format!("bm-row-{chat_id}-1")).is_none(), "the unstarred row is gone");
        assert!(window.find(format!("bm-row-{chat_id}-0")).visible(), "the other row stays");
        assert_eq!(ws.read(cx).active, 0, "unstarring never switches chats");
    });
    ws.read_with(cx, |ws, _| {
        let flags: Vec<bool> = ws.chats[0].messages.iter().map(|m| m.bookmarked).collect();
        assert_eq!(flags, [true, false], "only the clicked row lost its star");
    });
}

/// "Clear all" unstars everything across chats; the panel falls back to the
/// empty state.
#[test]
fn clear_all_empties_the_panel() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "starred in chat one");
    star(&ws, cx, 0);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.new_chat(cx));
    });
    push(&ws, cx, Role::Assistant, "starred in chat two");
    star(&ws, cx, 0);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.bookmarks_panel.open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.click("clear-bookmarks", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("bookmarks-empty").visible(), "the empty state returns");
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.chats.iter().flat_map(|c| c.messages.iter()).all(|m| !m.bookmarked), "no stars survive");
    });
}

/// Nothing starred: the panel still renders with the muted hint instead of
/// an empty list.
#[test]
fn panel_empty_state() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.bookmarks_panel.open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("bookmarks-empty").label(), Some("No bookmarks"));
    });
}

/// Cmd-Shift-B toggles the panel; the header's × closes it.
#[test]
fn panel_toggles_via_keybinding_and_close_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([KeyBinding::new("cmd-shift-b", crate::chat_msg::bookmarks_panel::ToggleBookmarks, Some("workspace"))]);
        window.draw(cx).clear(cx);
        assert!(window.try_find("bookmarks-panel").is_none(), "panel starts closed");

        window.press("cmd-shift-b", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("bookmarks-panel").visible(), "cmd-shift-b opens the panel");
        assert!(ws.read(cx).bookmarks_panel.open);

        window.click("close-bookmarks", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("bookmarks-panel").is_none(), "close button hides the panel");
        assert!(!ws.read(cx).bookmarks_panel.open);
    });
}

/// The open flag round-trips through settings.json: toggling writes it, a
/// fresh workspace restores it.
#[test]
fn panel_open_state_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.toggle_bookmarks_panel(cx));
    });
    assert!(crate::persist::load_settings().bookmarks_panel_open, "toggle writes the setting");

    let (ws2, cx2) = app.add_window_view(Workspace::new);
    ws2.read_with(cx2, |ws, _| {
        assert!(ws.bookmarks_panel.open, "panel reopens on launch");
    });
}
