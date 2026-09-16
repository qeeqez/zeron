//! Tests for the global-search context line — the neighboring message's
//! role + truncated text each result row shows under the match snippet —
//! and for the hit → message-index plumbing `open_hit` lands on.
//! Declared from `global_search.rs` via `#[path]` — `main.rs` is at the
//! SLOC cap.

use std::rc::Rc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext};

use crate::chat_search::context_line;
use crate::global_search::{SearchDoc, SearchFilters, search};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

fn msg(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

/// One searchable doc from `(role, text)` pairs — distinct ascending
/// timestamps so newest-first ranking is stable.
fn doc(msgs: &[(Role, &str)]) -> SearchDoc {
    let base = std::time::SystemTime::now();
    let messages = msgs
        .iter()
        .enumerate()
        .map(|(ix, (role, t))| ChatMessage {
            at: base + std::time::Duration::from_secs(ix as u64),
            ..msg(*role, t)
        })
        .collect();
    SearchDoc {
        chat_id: Some(1),
        file_ix: 0,
        title: "Chat".into(),
        provider: String::new(),
        model: String::new(),
        messages: Rc::new(messages),
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-gsearch-ctx-test-{}", std::process::id()));
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
fn push_to(this: &mut Workspace, role: Role, s: &str) {
    std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(msg(role, s));
}

#[test]
fn context_line_shows_the_prompt_for_assistant_hits() {
    let msgs = vec![msg(Role::User, "how do I exit vim"), msg(Role::Assistant, "press escape then :q")];
    let (role, text) = context_line(&msgs, 1).expect("a middle message has context");
    assert_eq!(role, Role::User, "an assistant hit anchors on its prompt");
    assert_eq!(text.as_ref(), "how do I exit vim");
}

#[test]
fn context_line_shows_the_reply_for_user_hits() {
    let msgs = vec![msg(Role::User, "how do I exit vim"), msg(Role::Assistant, "press escape then :q")];
    let (role, text) = context_line(&msgs, 0).expect("the first message falls back to its reply");
    assert_eq!(role, Role::Assistant);
    assert_eq!(text.as_ref(), "press escape then :q");
}

#[test]
fn context_line_edges_and_empty() {
    let single = vec![msg(Role::User, "alone")];
    assert_eq!(context_line(&single, 0), None, "the only message has no neighbor");
    assert_eq!(context_line(&single, 5), None, "out of range is no context");
    let tail = vec![msg(Role::User, "first"), msg(Role::Assistant, "last")];
    let (role, _) = context_line(&tail, 1).expect("the last message uses the previous one");
    assert_eq!(role, Role::User);
}

#[test]
fn context_line_collapses_and_clips() {
    let long = "x".repeat(200);
    let msgs = vec![msg(Role::User, &format!("{long}\n\nsecond   paragraph")), msg(Role::Assistant, "hit")];
    let (_, text) = context_line(&msgs, 1).unwrap();
    assert!(text.ends_with('…'), "long context is clipped: {text}");
    assert!(text.chars().count() <= 73, "clip budget plus ellipsis: {}", text.chars().count());
    assert!(!text.contains('\n'), "context is one line");
}

#[test]
fn hits_carry_context_and_message_index() {
    let docs = vec![doc(&[
        (Role::User, "where is the needle"),
        (Role::Assistant, "the needle is here"),
        (Role::User, "unrelated"),
    ])];
    let hits = search(&docs, "needle", &SearchFilters::default());
    assert_eq!(hits.len(), 2);
    // Newest first: the assistant hit, then the user hit.
    assert_eq!(hits[0].msg_ix, 1);
    let (role, text) = hits[0].context.clone().expect("assistant hit carries its prompt");
    assert_eq!(role, Role::User);
    assert!(text.contains("where is the needle"), "context names the prompt: {text}");
    assert_eq!(hits[1].msg_ix, 0);
    let (role, text) = hits[1].context.clone().expect("user hit carries its reply");
    assert_eq!(role, Role::Assistant);
    assert!(text.contains("the needle is here"), "context names the reply: {text}");
    // The plumbing `open_hit` relies on: msg_ix indexes the matching
    // message itself.
    for hit in &hits {
        let m = &docs[0].messages[hit.msg_ix];
        assert!(crate::chat_search::msg_matches(m, "needle"), "msg_ix points at a match");
    }
}

#[test]
fn dialog_row_shows_context_and_confirm_lands_highlighted() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            push_to(this, Role::User, "where is the needle");
            push_to(this, Role::Assistant, "the needle is here");
            // `push_to` bypasses the send path — size the scroller like a
            // real send would so `scroll_to_item` has rows to land on.
            let count = this.filtered_count(cx);
            this.scroller.update(cx, |s, cx| s.reset(count, cx));
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("cmd-shift-f", cx);
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.global_search.update(cx, |state, cx| state.set_query("needle", window, cx));
        });
        window.draw(cx).clear(cx);
        // Row 0 is the assistant hit — its context line names the prompt.
        assert_eq!(window.find(("hit-context", 0usize)).label(), Some("You: where is the needle"));
        // Row 1 is the user hit — its context line names the reply.
        assert_eq!(window.find(("hit-context", 1usize)).label(), Some("Rixl: the needle is here"));
        // Confirm row 0: the chat opens scrolled to the highlighted match.
        ws.read(cx).global_search.read(cx).focus_handle(cx).dispatch_action(
            &gpui_kit::component::dialog::Confirm { secondary: false },
            window,
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).find.open, "the find bar opens on the query");
        assert_eq!(window.find(("find-hit", 1usize)).label(), Some("current find match"));
        assert!(window.find(("msg", 1usize)).visible(), "the matched message is scrolled into view");
    });
}

#[test]
fn open_hit_restores_full_chat_state() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            // A disk-only chat carrying fields the old per-field loader
            // dropped — folder, color, instructions, prompt history.
            let dir = this.project.chats_dir();
            std::fs::create_dir_all(&dir).unwrap();
            let json = serde_json::json!({
                "v": 1,
                "title": "Disk chat",
                "messages": [{ "role": "User", "kind": { "Text": "disk needle" } }],
                "folder": "Work",
                "color": "red",
                "instructions": "be terse",
                "prompt_history": ["earlier prompt"],
            });
            std::fs::write(dir.join(format!("{}.json", this.chats.len())), serde_json::to_string(&json).unwrap()).unwrap();
            let hits = search(&this.search_docs(), "needle", &SearchFilters::default());
            assert_eq!(hits.len(), 1);
            this.open_hit(&hits[0], "needle", window, cx);
            let chat = &this.chats[this.active];
            assert_eq!(chat.folder, "Work", "the folder survives the single-file load");
            assert!(chat.color.is_some(), "the color tag survives");
            assert_eq!(chat.instructions.as_deref(), Some("be terse"), "custom instructions survive");
            assert_eq!(chat.prompt_history, ["earlier prompt"], "prompt history survives");
        });
    });
}
