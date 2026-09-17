//! Tests for the global-search Match Case / Whole Word chips — pure
//! tests cover `search` under each flag combo and the snippet's hit
//! alignment (the excerpt window must land on the text that actually
//! matched, not the first folded look-alike); a headless test drives the
//! filter row's toggle chips.
//! Declared from `global_search.rs` via `#[path]` — `main.rs` is at the
//! SLOC cap.

use std::rc::Rc;

use gpui_kit::component::Root;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::chat_search::find_opts::FindOpts;
use crate::global_search::{SearchDoc, SearchFilters, search};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

const CASE: FindOpts = FindOpts { case_sensitive: true, whole_word: false };
const WORD: FindOpts = FindOpts { case_sensitive: false, whole_word: true };
const BOTH: FindOpts = FindOpts { case_sensitive: true, whole_word: true };

fn filters(opts: FindOpts) -> SearchFilters {
    SearchFilters { opts, ..SearchFilters::default() }
}

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

fn doc(texts: &[&str]) -> SearchDoc {
    doc_at(0, texts)
}

fn doc_at(file_ix: usize, texts: &[&str]) -> SearchDoc {
    SearchDoc {
        chat_id: Some(1),
        file_ix,
        title: "Chat".into(),
        provider: String::new(),
        model: String::new(),
        messages: Rc::new(texts.iter().map(|t| msg(t)).collect()),
    }
}

#[test]
fn match_case_narrows_hits() {
    let docs = vec![doc(&["Hello there", "hello again", "HELLO!"])];
    assert_eq!(search(&docs, "hello", &SearchFilters::default()).len(), 3, "default folds case");
    let hits = search(&docs, "hello", &filters(CASE));
    assert_eq!(hits.len(), 1, "exact case only");
    assert_eq!(hits[0].msg_ix, 1);
    assert!(hits[0].snippet.contains("hello"), "snippet carries the cased hit: {}", hits[0].snippet);
}

#[test]
fn whole_word_requires_boundaries() {
    // One message per doc — PER_CHAT's cap would otherwise hide hits.
    let docs = vec![doc_at(0, &["hitter"]), doc_at(1, &["a hit."]), doc_at(2, &["a_hit"]), doc_at(3, &["hit-or-miss"])];
    assert_eq!(search(&docs, "hit", &SearchFilters::default()).len(), 4, "substring matches all");
    let hits = search(&docs, "hit", &filters(WORD));
    assert_eq!(hits.len(), 2, "word chars block 'hitter' and 'a_hit'; '-' is a boundary");
    assert!(hits.iter().all(|h| [1, 3].contains(&h.file_ix)), "hits are 'a hit.' and 'hit-or-miss'");
}

#[test]
fn toggles_combine() {
    let docs = vec![doc(&["Hit me", "a hit.", "Hit Parade"])];
    let hits = search(&docs, "Hit", &filters(BOTH));
    assert_eq!(hits.len(), 2, "case-exact whole words only: 'Hit me', 'Hit Parade'");
    assert!(hits.iter().all(|h| [0, 2].contains(&h.msg_ix)));
}

#[test]
fn snippet_centers_on_the_case_exact_hit() {
    // The folded first occurrence sits far before the cased one — the
    // excerpt must window the hit `search` matched, not the fold's.
    let long = format!("{} needle {} NEEDLE {}", "x".repeat(80), "y".repeat(80), "z".repeat(80));
    let docs = vec![doc(&[&long])];
    let hits = search(&docs, "NEEDLE", &filters(CASE));
    assert_eq!(hits.len(), 1);
    let snippet = &hits[0].snippet;
    assert!(snippet.contains("NEEDLE"), "excerpt centers on the cased hit: {snippet}");
    assert!(!snippet.contains("needle"), "the lowercase look-alike is out of window: {snippet}");
}

#[test]
fn snippet_centers_on_the_whole_word_hit() {
    let long = format!("hitter {} hit {}", "y".repeat(80), "z".repeat(80));
    let docs = vec![doc(&[&long])];
    let hits = search(&docs, "hit", &filters(WORD));
    assert_eq!(hits.len(), 1);
    let snippet = &hits[0].snippet;
    assert!(snippet.contains("hit"), "{snippet}");
    assert!(!snippet.contains("hitter"), "the substring inside 'hitter' isn't the hit: {snippet}");
    // The default substring scan still excerpts the first hit — inside
    // "hitter" — the contrast pins the window to the real match.
    let folded = search(&docs, "hit", &SearchFilters::default());
    assert!(folded[0].snippet.contains("hitter"), "{}", folded[0].snippet);
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-gopts-test-{}", std::process::id()));
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

#[test]
fn toggle_chips_narrow_results_and_stick() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(msg("Hello there"));
            std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(msg("hello again"));
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("cmd-shift-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "cmd-shift-f opens the search dialog");
        ws.update(cx, |this, cx| {
            this.global_search.update(cx, |state, cx| state.set_query("hello", window, cx));
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "both case variants match");
        assert_eq!(window.find("search-match-case").checked(), Some(false), "Match Case starts off");
        assert_eq!(window.find("search-whole-word").checked(), Some(false), "Whole Word starts off");
    });
    // The click dispatches a real mouse event at the chip's bounds — the
    // dialog's enter animation runs off the wall clock, so wait it out
    // like `click_menu_item` does for popovers or the point lands on the
    // still-animating scrim instead of the chip.
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(700));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "dialog still up before the click");
        window.click("search-match-case", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).search_filters.read(cx).opts.case_sensitive, "the chip writes the filters entity");
        assert_eq!(window.find("search-match-case").checked(), Some(true), "the chip reads on");
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "only the lowercase hit survives");
        // The filters entity outlives the dialog — toggles stick for the
        // session like the find bars' do.
        window.close_dialog(cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("command").is_none(), "dialog closed");
        ws.update(cx, |this, cx| this.open_global_search(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "reopened");
        assert_eq!(window.find("search-match-case").checked(), Some(true), "Match Case stays set for the session");
    });
}
