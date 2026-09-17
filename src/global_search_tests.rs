//! Tests for `global_search` — the Cmd-Shift-F cross-chat search. Pure
//! tests cover matching, ranking and snippets over `SearchDoc`s; headless
//! tests mount a workspace, write chat files beside the loaded chats, and
//! drive the real dialog (open, type, confirm, jump).

use std::rc::Rc;

use gpui_kit::component::IndexPath;
use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext};

use crate::global_search::{SearchDoc, SearchFilters, SearchHit, search};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

fn msg(text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::User,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn doc(chat_id: Option<u64>, file_ix: usize, title: &str, texts: &[&str]) -> SearchDoc {
    // Distinct timestamps — recency ranking needs a stable order.
    let base = std::time::SystemTime::now();
    let messages = texts
        .iter()
        .enumerate()
        .map(|(ix, t)| ChatMessage {
            alternatives: vec![],
            at: base + std::time::Duration::from_secs((file_ix * 1000 + ix) as u64),
            ..msg(t)
        })
        .collect();
    SearchDoc {
        chat_id,
        file_ix,
        title: title.into(),
        provider: String::new(),
        model: String::new(),
        messages: Rc::new(messages),
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-gsearch-test-{}", std::process::id()));
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

/// Append a text message to the active chat — works inside `cx.update`
/// where only `&mut App` is available.
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

#[test]
fn search_matches_across_chats() {
    let docs = vec![
        doc(Some(1), 0, "First chat", &["hello world", "nothing here"]),
        doc(Some(2), 1, "Second chat", &["say hello again", "unrelated"]),
        doc(Some(3), 2, "Third chat", &["no match"]),
    ];
    let hits = search(&docs, "hello", &SearchFilters::default());
    assert_eq!(hits.len(), 2, "one hit per matching chat");
    assert_eq!(hits[0].title.as_ref(), "Second chat", "newest message ranks first");
    assert_eq!(hits[0].msg_ix, 0);
    assert_eq!(hits[0].chat_id, Some(2));
    assert!(hits[0].snippet.contains("hello"), "snippet carries the match: {}", hits[0].snippet);
    assert_eq!(hits[1].title.as_ref(), "First chat");
}

#[test]
fn search_empty_query_returns_nothing() {
    let docs = vec![doc(Some(1), 0, "Chat", &["hello"])];
    assert!(search(&docs, "", &SearchFilters::default()).is_empty());
    assert!(search(&docs, "   ", &SearchFilters::default()).is_empty(), "whitespace-only is still empty");
}

#[test]
fn search_caps_results_per_chat() {
    let texts: Vec<String> = (0..10).map(|i| format!("hit number {i}")).collect();
    let docs = vec![doc(Some(1), 0, "Busy", &texts.iter().map(String::as_str).collect::<Vec<_>>())];
    let hits = search(&docs, "hit", &SearchFilters::default());
    assert_eq!(hits.len(), 3, "PER_CHAT keeps the newest three matches");
    assert_eq!(hits[0].msg_ix, 9, "newest message first");
}

#[test]
fn snippet_centers_on_match() {
    let long = format!("{} needle {}", "x".repeat(120), "y".repeat(120));
    let docs = vec![doc(Some(1), 0, "Chat", &[&long])];
    let hits = search(&docs, "needle", &SearchFilters::default());
    let snippet = &hits[0].snippet;
    assert!(snippet.starts_with('…') && snippet.ends_with('…'), "clipped both sides: {snippet}");
    assert!(snippet.contains("needle"));
    assert!(snippet.len() < long.len());
}

#[test]
fn search_docs_include_disk_only_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, _cx| {
            push_to(this, "loaded needle");
            // A chat file this window never loaded — index past the live set.
            write_chat_file(&this.project.chats_dir(), this.chats.len(), "Disk chat", &["disk needle"]);
            let docs = this.search_docs();
            assert_eq!(docs.len(), 2, "loaded chat plus the on-disk file");
            assert_eq!(docs[1].chat_id, None);
            assert_eq!(docs[1].title.as_ref(), "Disk chat");
            let hits = search(&docs, "needle", &SearchFilters::default());
            assert_eq!(hits.len(), 2, "both loaded and disk chats match");
            assert!(hits.iter().any(|h| h.chat_id.is_none() && h.file_ix == 1));
        });
    });
}

#[test]
fn open_hit_selects_chat_and_jumps_to_message() {
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
            // The hit: chat 0, message index 2 ("needle two").
            let hit = SearchHit {
                chat_id: Some(this.chats[0].id),
                file_ix: 0,
                msg_ix: 2,
                title: this.chats[0].title.clone(),
                snippet: "needle two".into(),
                context: None,
                provider: String::new(),
                model: String::new(),
                at: std::time::SystemTime::now(),
            };
            this.open_hit(&hit, "needle", window, cx);
            assert_eq!(this.active, 0, "the hit's chat is selected");
            assert!(this.find.open, "the find bar opens on the query");
            assert_eq!(this.find.match_ix, 1, "lands on the second match");
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("2 / 2"));
        assert_eq!(window.find(("find-hit", 2usize)).label(), Some("current find match"));
    });
}

#[test]
fn open_hit_loads_disk_only_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            write_chat_file(&this.project.chats_dir(), this.chats.len(), "Disk chat", &["disk needle"]);
            let docs = this.search_docs();
            let hits = search(&docs, "needle", &SearchFilters::default());
            assert_eq!(hits.len(), 1);
            this.open_hit(&hits[0], "needle", window, cx);
            assert_eq!(this.chats.len(), 2, "the file loads as a live chat");
            assert_eq!(this.active, 1);
            assert_eq!(this.chats[1].title.as_ref(), "Disk chat");
            assert!(this.find.open);
            assert_eq!(this.find.match_ix, 0);
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("1 / 1"));
    });
}

#[test]
fn cmd_shift_f_opens_lists_and_confirms() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            push_to(this, "needle in first chat");
            this.new_chat(cx);
            push_to(this, "needle in second chat");
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("cmd-shift-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "cmd-shift-f opens the search dialog");
        // Empty query: no results, just the hint.
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 0);
        ws.update(cx, |this, cx| {
            this.global_search.update(cx, |state, cx| state.set_query("needle", window, cx));
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "both chats match");
        // Row 1 is the older chat's hit — confirming it switches chats.
        ws.update(cx, |this, cx| {
            this.global_search
                .update(cx, |state, cx| state.set_selected_index(Some(IndexPath::new(1).section(0)), window, cx));
        });
        ws.read(cx)
            .global_search
            .read(cx)
            .focus_handle(cx)
            .dispatch_action(&Confirm { secondary: false }, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).active, 0, "confirming opens the hit's chat");
        assert!(ws.read(cx).find.open, "the find bar jumps to the match");
        assert!(window.try_find("command").is_none(), "the dialog closes on confirm");
    });
}

#[test]
fn sidebar_row_opens_global_search() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("search-all-chats", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "the sidebar row opens the dialog");
    });
    let _ = ws;
}
