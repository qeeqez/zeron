//! Tests for the find bar's Match Case / Whole Word toggles — pure tests
//! cover `FindOpts` matching (case folding, word boundaries, unicode);
//! headless tests drive the real bar's chips, counter and marks.
//! Declared from `chat_find.rs` via `#[path]` — `main.rs` is at the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::chat_find::matching_messages;
use crate::chat_search::find_opts::FindOpts;
use crate::chat_search::role_filter::RoleFilter;
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

fn text(s: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Text(s.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

const CASE: FindOpts = FindOpts { case_sensitive: true, whole_word: false };
const WORD: FindOpts = FindOpts { case_sensitive: false, whole_word: true };
const BOTH: FindOpts = FindOpts { case_sensitive: true, whole_word: true };

#[test]
fn match_case_compares_exact_case() {
    assert!(FindOpts::default().text_matches("Hello", "hello"), "default folds both sides");
    assert!(!CASE.text_matches("Hello", "hello"));
    assert!(CASE.text_matches("Hello", "Hello"));
    assert!(CASE.text_matches("say Hello", "Hello"), "substring still applies");
    let messages = vec![text("Hello world"), text("hello again")];
    assert_eq!(matching_messages(&messages, "hello", RoleFilter::All, CASE), vec![1], "exact case only");
    assert_eq!(matching_messages(&messages, "Hello", RoleFilter::All, CASE), vec![0]);
}

#[test]
fn whole_word_requires_non_word_boundaries() {
    assert!(WORD.text_matches("a hit.", "hit"), "punctuation is a boundary");
    assert!(WORD.text_matches("hit", "hit"), "text edges count as boundaries");
    assert!(WORD.text_matches("(hit)", "hit"));
    assert!(WORD.text_matches("hit-or-miss", "hit"), "hyphen is a boundary");
    assert!(!WORD.text_matches("hitter", "hit"), "a trailing letter blocks the hit");
    assert!(!WORD.text_matches("a_hit", "hit"), "underscore is a word char");
    assert!(!WORD.text_matches("hit2", "hit"), "digits are word chars");
    assert!(!WORD.text_matches("sprint hits", "hit"), "plural isn't a word hit");
}

#[test]
fn whole_word_is_unicode_aware() {
    assert!(!WORD.text_matches("café", "caf"), "é is a letter — caf is not a word here");
    assert!(WORD.text_matches("le café noir", "café"));
    assert!(WORD.text_matches("über cool", "über"), "unicode needle matches whole");
    assert!(!WORD.text_matches("übercool", "über"));
}

#[test]
fn toggles_combine_and_apply_to_every_haystack() {
    assert!(BOTH.text_matches("a Hit.", "Hit"));
    assert!(!BOTH.text_matches("a hit.", "Hit"), "case still applies under whole-word");
    assert!(!BOTH.text_matches("Hitter", "Hit"));
    // Tool messages match on name/detail/output — the flags gate them all.
    let tool = ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Tool(crate::model::ToolCall {
            tool_ix: 0,
            name: "Shell".into(),
            detail: "run tests".into(),
            output: "ok".into(),
            status: crate::model::ToolStatus::Done,
            expanded: false,
        }),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    };
    let messages = vec![tool];
    assert_eq!(matching_messages(&messages, "shell", RoleFilter::All, FindOpts::default()), vec![0]);
    assert!(matching_messages(&messages, "shell", RoleFilter::All, CASE).is_empty(), "tool name folds away too");
    assert_eq!(matching_messages(&messages, "Shell", RoleFilter::All, CASE), vec![0]);
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-findtoggle-test-{}", std::process::id()));
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

/// Focus the composer and open the find bar with `query` typed.
fn open_find(ws: &Entity<Workspace>, cx: &mut VisualTestContext, query: &str) {
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("find-bar").visible(), "cmd-f should open the find bar");
    });
    // The deferred focus lands between updates; typing then fills the input.
    cx.update(|window, cx| {
        window.input(query, cx);
        window.draw(cx).clear(cx);
    });
}

#[test]
fn toggle_chips_narrow_and_restore_hits() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "Hit me");
    push(&ws, cx, Role::Assistant, "a hit.");
    push(&ws, cx, Role::Assistant, "hitter");
    open_find(&ws, cx, "hit");
    cx.update(|window, cx| {
        assert_eq!(window.find("find-count").label(), Some("1 / 3"), "insensitive substring matches all three");
        assert_eq!(window.find("find-match-case").checked(), Some(false), "Match Case starts off");
        assert_eq!(window.find("find-whole-word").checked(), Some(false), "Whole Word starts off");
        window.click("find-match-case", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-match-case").checked(), Some(true), "the chip reads on");
        assert_eq!(window.find("find-count").label(), Some("1 / 2"), "only lowercase hits survive");
        assert!(window.try_find(("find-hit", 0usize)).is_none(), "the capitalized hit loses the mark");
        assert_eq!(window.find(("find-hit", 1usize)).label(), Some("current find match"));
        window.click("find-whole-word", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-whole-word").checked(), Some(true));
        assert_eq!(window.find("find-count").label(), Some("1 / 1"), "whole word drops 'hitter'");
        assert!(window.try_find(("find-hit", 2usize)).is_none());
        assert_eq!(window.find(("find-hit", 1usize)).label(), Some("current find match"));
        window.click("find-match-case", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("1 / 2"), "case off: 'Hit me' returns, 'hitter' stays out");
        window.click("find-whole-word", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("1 / 3"), "both off restores every hit");
    });
}

#[test]
fn toggles_stay_set_across_close_and_reopen() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "find me");
    open_find(&ws, cx, "find");
    cx.update(|window, cx| {
        window.click("find-match-case", cx);
        window.click("find-close", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).find.open);
    });
    // The deferred composer focus lands between updates; cmd-f reopens.
    cx.update(|window, cx| {
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("find-bar").visible(), "cmd-f reopens the find bar");
        assert_eq!(window.find("find-match-case").checked(), Some(true), "Match Case stays sticky for the session");
        assert_eq!(window.find("find-whole-word").checked(), Some(false));
    });
}

#[test]
fn jump_to_message_widens_toggles_that_hide_the_target() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "HIT ME");
    push(&ws, cx, Role::Assistant, "a hit.");
    open_find(&ws, cx, "hit");
    cx.update(|window, cx| {
        window.click("find-match-case", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("1 / 1"), "only the lowercase hit survives");
        // Global search confirmed a hit in message 0 under its
        // case-insensitive matching — the toggles must not hide it.
        ws.update(cx, |this, cx| this.jump_to_message("hit", 0, window, cx));
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).find.opts.case_sensitive, "Match Case resets to reach the target");
        assert_eq!(ws.read(cx).find.match_ix, 0);
        assert_eq!(window.find(("find-hit", 0usize)).label(), Some("current find match"));
        assert_eq!(window.find("find-count").label(), Some("1 / 2"));
    });
}
