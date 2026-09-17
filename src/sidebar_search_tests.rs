//! Tests for `sidebar_search` — the chat-list field's message-body scan.
//! Pure tests cover `chat_hit` (count, newest match, snippet); headless
//! tests mount a workspace, type into the real sidebar field, and check
//! the "Messages" group: body-only hits, click-to-jump, the "+N more"
//! footer, and the empty-query hide. Declared from `sidebar_search.rs` via
//! `#[path]` — `main.rs` is at the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use super::{MAX_ROWS, chat_hit};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

fn msg(text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::User,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-sidebar-search-test-{}", std::process::id()));
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

/// Append a text message to the active chat.
fn push_to(this: &mut Workspace, s: &str) {
    std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(msg(s));
}

/// Write a minimal `N.json` chat file — the shape `persist::save_chats`
/// emits — straight into the project's chats dir.
fn write_chat_file(dir: &std::path::Path, ix: usize, title: &str, texts: &[&str]) {
    let messages: Vec<serde_json::Value> = texts.iter().map(|t| serde_json::json!({ "role": "User", "kind": { "Text": t } })).collect();
    let json = serde_json::json!({ "v": 1, "title": title, "messages": messages });
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{ix}.json")), serde_json::to_string(&json).unwrap()).unwrap();
}

/// Focus the sidebar search field and type `text` — each character is a
/// real keystroke, so the debounced disk scan re-arms per char.
fn type_query(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App, text: &str) {
    ws.update(cx, |this, cx| {
        this.search.update(cx, |s, cx| s.focus(window, cx));
    });
    window.input(text, cx);
}

/// Run the debounce + background scan to completion.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
}

#[test]
fn chat_hit_counts_and_picks_newest() {
    let messages = vec![msg("needle one"), msg("unrelated"), msg("needle two")];
    let hit = chat_hit(Some(7), 0, "Chat".into(), &messages, "needle").expect("two messages match");
    assert_eq!(hit.count, 2);
    assert_eq!(hit.msg_ix, 2, "the newest match is the click target");
    assert!(hit.snippet.contains("needle two"));
    assert_eq!(hit.chat_id, Some(7));
}

#[test]
fn chat_hit_none_without_match() {
    let messages = vec![msg("nothing here")];
    assert!(chat_hit(Some(1), 0, "Chat".into(), &messages, "needle").is_none());
    assert!(chat_hit(Some(1), 0, "Chat".into(), &messages, "").is_none(), "empty query matches nothing");
}

/// A title match keeps the chat in the list AND surfaces its body hits
/// under Messages — the two searches are independent.
#[test]
fn title_and_body_both_match() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, _cx| {
            this.chats[0].title = "needle chat".into();
            push_to(this, "body has needle too");
        });
        type_query(&ws, window, cx, "needle");
    });
    // The Change subscription runs at the update boundary — re-enter to draw.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Title match: the chat row stays in the list.
        let chat_id = ws.read(cx).chats[0].id;
        assert!(window.find(("chat-row", chat_id)).visible(), "title match keeps the chat row");
        // Body match: the same chat also lists under Messages.
        assert!(window.find("group-header-Messages").visible(), "Messages group renders");
        assert!(window.find(("sidebar-hit", 0usize)).visible(), "the body hit row renders");
        assert_eq!(window.find(("sidebar-hit-count", 0usize)).label(), Some("1"));
    });
}

/// A chat whose title doesn't match still lists under Messages when a
/// body does — including a chat file this window never loaded.
#[test]
fn body_only_match_lists_under_messages() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, _cx| {
            this.chats[0].title = "unrelated title".into();
            push_to(this, "the needle is in the body");
            // A chat file this window never loaded — index past the live set.
            write_chat_file(&this.project.chats_dir(), this.chats.len(), "Old chat", &["disk needle"]);
        });
        type_query(&ws, window, cx, "needle");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // No title matches — the placeholder shows — but the live chat's
        // body hit is already listed (live scan is synchronous).
        assert!(window.find("sidebar-no-match").visible(), "no chat titles match");
        assert!(window.find(("sidebar-hit", 0usize)).visible(), "live body hit renders immediately");
    });
    // The disk chat lands after the debounce.
    settle(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("sidebar-hit", 1usize)).visible(), "disk-only chat hit lands after the debounce");
        assert_eq!(window.find(("sidebar-hit-count", 1usize)).label(), Some("1"));
    });
}

/// Clicking a Messages row opens the chat and lands the find bar on the
/// matched message — the same path the search dialog's confirm takes.
#[test]
fn click_opens_chat_at_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            push_to(this, "no match");
            push_to(this, "needle one");
            push_to(this, "needle two");
            this.new_chat(cx);
            push_to(this, "other chat");
            assert_eq!(this.active, 1);
        });
        type_query(&ws, window, cx, "needle");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("sidebar-hit", 0usize), cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).active, 0, "the hit's chat is selected");
        assert!(ws.read(cx).find.open, "the find bar opens on the query");
        assert_eq!(ws.read(cx).find.match_ix, 1, "lands on the newest match");
        assert_eq!(window.find("find-count").label(), Some("2 / 2"));
    });
}

/// Past MAX_ROWS the group stops growing and a "+N more" footer row
/// appears; clicking it opens the full search dialog with the query.
#[test]
fn hits_cap_with_more_footer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let extra = 5;
    cx.update(|window, cx| {
        ws.update(cx, |this, _cx| {
            // Disk-only chats past the live set — each one a hit.
            for ix in 0..(MAX_ROWS + extra) {
                write_chat_file(&this.project.chats_dir(), this.chats.len() + ix, &format!("Chat {ix}"), &["needle"]);
            }
        });
        type_query(&ws, window, cx, "needle");
    });
    settle(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).sidebar_hits.len(), MAX_ROWS, "the group caps at MAX_ROWS");
        assert_eq!(ws.read(cx).sidebar_hits_extra, extra);
        assert!(window.try_find("sidebar-hits-more").is_some(), "+5 more footer row renders");
        ws.update(cx, |this, cx| this.open_global_search_seeded("needle", window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "+N more opens the full search dialog");
        assert_eq!(ws.read(cx).global_search.read(cx).query(cx).as_ref(), "needle", "the query carries over");
    });
}

/// No query, no group — and clearing the field removes it again.
#[test]
fn empty_query_hides_the_group() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, _cx| push_to(this, "needle in body"));
        window.draw(cx).clear(cx);
        assert!(window.try_find("group-header-Messages").is_none(), "no query, no group");
        type_query(&ws, window, cx, "needle");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("group-header-Messages").visible(), "a body match shows the group");
    });
    // The ✕ button clears the field — `set_value` emits no Change, but the
    // group renders only while the query is non-empty.
    cx.update(|window, cx| {
        window.click("search-clear", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("group-header-Messages").is_none(), "clearing the query hides the group");
    });
}
