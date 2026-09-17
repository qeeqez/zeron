//! Headless UI tests for the activity center: the bell's unread badge, the
//! panel's row actions (open chat, scroll to approval, per-row dismiss),
//! and Clear-all. Helpers live in `activity_tests` — same pattern as
//! `changes_ui_tests` sharing `mount` with its sibling test files.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use crate::activity_tests::{AskBackend, OkBackend, mount, send_reply};

#[test]
fn badge_counts_until_its_chat_opens() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("activity-bell").visible(), "the bell renders in the top bar");
        assert!(window.find("activity-badge").visible(), "one unread entry shows the badge");
        assert!(window.try_find("activity-panel").is_none(), "panel starts closed");

        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("activity-panel").visible(), "clicking the bell opens the panel");
        assert!(window.find(("activity-entry", 0usize)).visible(), "the entry row renders");
        assert!(window.find(("activity-unread", 0usize)).visible(), "an unread row carries the dot");
        assert!(window.find("activity-badge").visible(), "viewing the list keeps the badge");
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.activity_open);
        assert_eq!(ws.activity.unread_count(), 1, "opening the panel alone doesn't mark read");
    });
}

#[test]
fn entry_click_opens_its_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    // A second chat takes focus; the entry must lead back to chat 0.
    cx.update(|_window, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    assert_eq!(ws.read_with(cx, |ws, _| ws.active), 1);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        window.click(("activity-entry", 0usize), cx);
        window.draw(cx).clear(cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, 0, "clicking the entry selects its chat");
        assert!(!ws.activity_open, "the panel closes after the click");
        assert_eq!(ws.activity.unread_count(), 0, "opening the chat clears the row's dot");
    });
}

#[test]
fn entry_click_marks_only_its_chat_read() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    // A second chat takes focus and finishes its own turn — two unread
    // entries, one per chat.
    cx.update(|_window, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    let chat0_created = ws.read_with(cx, |ws, _| ws.chats[0].created_at);
    ws.read_with(cx, |ws, _| assert_eq!(ws.activity.unread_count(), 2));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        // Row ids are feed indices, not display order: entries[0] is chat
        // 0's entry, rendered last (newest-first).
        window.click(("activity-entry", 0usize), cx);
        window.draw(cx).clear(cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, 0, "the older row routes to its own chat");
        assert_eq!(ws.activity.unread_count(), 1, "only the opened chat's entries clear");
        assert_eq!(ws.activity.entries[0].chat_created, chat0_created);
        assert!(!ws.activity.entries[0].unread);
        assert!(ws.activity.entries[1].unread, "the other chat's entry stays unread");
    });
}

#[test]
fn dismiss_removes_row_without_opening() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    cx.update(|_window, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    let chat0_created = ws.read_with(cx, |ws, _| ws.chats[0].created_at);

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        // Feed index 1 is chat 1's entry — the top row.
        window.click(("activity-dismiss", 1usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("activity-entry", 1usize)).is_none(), "one row remains after the dismiss");
        assert!(window.find("activity-panel").visible(), "dismissing keeps the panel open");
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, 1, "dismissing doesn't navigate");
        assert_eq!(ws.activity.entries.len(), 1);
        assert_eq!(ws.activity.entries[0].chat_created, chat0_created, "the dismissed row is gone");
        assert_eq!(ws.activity.unread_count(), 1, "the surviving row keeps its dot");
    });
}

#[test]
fn approval_entry_scrolls_to_pending_card() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(AskBackend), false, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The card sits at message 1 under 40 filler messages — the virtual
        // scroller is tail-anchored, so it isn't rendered yet.
        assert!(window.try_find(("approval", 1usize)).is_none(), "pending card starts off-screen");
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        window.click(("activity-entry", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("approval", 1usize)).visible(), "clicking the entry scrolls the pending card into view");
    });
}

#[test]
fn mark_all_read_clears_dots_without_opening() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    cx.update(|_window, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("activity-unread", 0usize)).visible(), "an unread row carries the dot");
        window.click("activity-mark-read", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("activity-badge").is_none(), "the badge is gone");
        assert!(window.try_find(("activity-unread", 0usize)).is_none(), "no dots remain");
        assert!(window.find("activity-panel").visible(), "the panel stays open");
        assert!(window.try_find("activity-mark-read").is_none(), "with nothing unread the action hides");
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.activity.entries.len(), 2, "marking read keeps the rows");
        assert_eq!(ws.activity.unread_count(), 0);
        assert_eq!(ws.active, 1, "no chat was opened");
    });
}

#[test]
fn clear_read_drops_only_read_rows() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    cx.update(|_window, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    let (dir, chat1_created) = ws.read_with(cx, |ws, _| (ws.project.dir().to_path_buf(), ws.chats[1].created_at));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        // Opening chat 0's row marks it read — the feed is now mixed.
        window.click(("activity-entry", 0usize), cx);
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("activity-clear-read").visible(), "a mixed feed offers Clear read");
        window.click("activity-clear-read", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("activity-panel").visible(), "the panel stays open");
        assert!(window.find("activity-badge").visible(), "the unread row still badges the bell");
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.activity.entries.len(), 1, "the read row is swept");
        assert_eq!(ws.activity.entries[0].chat_created, chat1_created);
        assert!(ws.activity.entries[0].unread, "the unread row survives");
    });
    assert_eq!(crate::activity::ActivityFeed::load(&dir).entries.len(), 1, "the sweep persists to disk");
}

#[test]
fn clear_empties_feed_and_file() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_reply(&ws, std::sync::Arc::new(OkBackend), true, cx);
    let dir = ws.read_with(cx, |ws, _| ws.project.dir().to_path_buf());
    assert!(dir.join("activity.json").exists(), "recording persists the feed");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("activity-bell", cx);
        window.draw(cx).clear(cx);
        window.click("activity-clear", cx);
        window.draw(cx).clear(cx);
    });
    ws.read_with(cx, |ws, _| assert!(ws.activity.entries.is_empty(), "Clear empties the feed"));
    assert!(!dir.join("activity.json").exists(), "Clear removes the persisted feed");
}
