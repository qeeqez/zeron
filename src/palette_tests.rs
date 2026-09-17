//! Palette tests: pure fuzzy scoring plus headless UI coverage of the
//! dialog — filtering, chat navigation, and close-on-confirm.
//!
//! `#[gpui_kit::test]` and `use gpui_kit::*` crash the proc-macro on this
//! nightly, so tests use `TestAppContext::single()` under plain `#[test]`
//! with narrow imports (see ui_tests.rs).

use crate::palette_fuzzy::fuzzy_score;
use crate::palette_items::{Entry, entry_at};
use crate::workspace::Workspace;
use gpui_kit::component::IndexPath;
use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext};

/// Mount a `Workspace` in a headless window (same harness as ui_tests.rs).
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
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
fn fuzzy_subsequence_and_ranking() {
    // Subsequence, not substring: "nc" hits "New Chat" via word initials.
    assert!(fuzzy_score("nc", "New Chat").is_some());
    assert!(fuzzy_score("nc", "Toggle Sidebar").is_none());
    // Case-insensitive.
    assert!(fuzzy_score("NEW", "New Chat").is_some());
    // Out-of-order chars don't match.
    assert!(fuzzy_score("cn", "New Chat").is_none());
    // Tighter matches outrank scattered ones.
    assert!(fuzzy_score("chat", "New Chat") > fuzzy_score("chat", "Copy Transcript"));
    // Empty query matches everything at score 0.
    assert_eq!(fuzzy_score("", "anything"), Some(0));
    assert_eq!(fuzzy_score("  ", "anything"), Some(0));
}

#[test]
fn fuzzy_scores_best_alignment_not_first() {
    // A late contiguous run beats an early scattered match: the greedy
    // first-subsequence scan took the 'a' at index 0 and ate a 9-char gap
    // penalty instead of the contiguous "ab" at the end.
    assert_eq!(fuzzy_score("ab", "a.........ab"), fuzzy_score("ab", "ab"));
    assert!(fuzzy_score("ab", "a.........ab") > fuzzy_score("ab", "axxxxxxxxxyb"));
    // Word-start alignment still wins when a scattered match starts earlier.
    assert!(fuzzy_score("nc", "nonsense New Chat") > fuzzy_score("nc", "nonsense chat"));
}

#[test]
fn palette_opens_with_commands_and_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[this.active].title = "Palette Target".into();
            this.new_chat(cx);
            this.open_palette(window, cx);
        });
        window.draw(cx).clear(cx);
        // Commands group renders rows; the chats group is in the model even
        // where the virtual list clips it below the fold.
        assert!(window.find(IndexPath::new(0).section(0)).visible(), "commands should render");
        // 21 commands + 3 chats.
        assert_eq!(ws.read(cx).palette.read(cx).matched_count(), 24);
    });
}

#[test]
fn palette_fuzzy_filters_and_confirms_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[this.active].title = "Palette Target".into();
            this.new_chat(cx);
            this.chats[this.active].title = "Other".into();
            this.open_palette(window, cx);
        });
        window.draw(cx).clear(cx);
    });
    // Flush the deferred input focus so Confirm reaches the palette.
    cx.run_until_parked();
    cx.update(|window, cx| {
        // Type a fuzzy query — rebuild happens via the deferred on_query.
        ws.update(cx, |this, cx| {
            this.palette.update(cx, |state, cx| state.set_query("ptgt", window, cx));
        });
        window.draw(cx).clear(cx);
        let matched = ws.read(cx).palette.read(cx).matched_count();
        assert_eq!(matched, 1, "only the fuzzy-matching chat should remain");
        let selected = ws.read(cx).palette.read(cx).selected_index();
        assert_eq!(selected, Some(IndexPath::new(0).section(1)), "the chat row should be highlighted");

        // Enter confirms the highlighted chat and closes the dialog.
        ws.read(cx)
            .palette
            .read(cx)
            .focus_handle(cx)
            .dispatch_action(&Confirm { secondary: false }, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[ws.read(cx).active].title, "Palette Target");
        assert!(window.try_find("command").is_none(), "palette should close on confirm");
    });
}

#[test]
fn palette_confirm_runs_command() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_palette(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    let chats_before = cx.update(|window, cx| {
        let before = ws.read(cx).chats.len();
        // First row is "New Chat" — confirm it.
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
        before
    });
    // dispatch_action → confirm → NewChat dispatch are all deferred.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats.len(), chats_before + 1, "New Chat should run");
        assert!(window.try_find("command").is_none(), "palette should close on confirm");
    });
}

#[test]
fn entry_at_maps_paths() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        let chats = ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[this.active].title = "Second".into();
            this.palette_chats()
        });
        // Section 0 row 0 is the first command; section 1 row 0 is the
        // most recent chat in sidebar order.
        match entry_at(&chats, "", IndexPath::new(0).section(0), 0) {
            Some(Entry::Command(spec)) => assert_eq!(spec.label, "New Chat"),
            _ => panic!("expected the New Chat command"),
        }
        match entry_at(&chats, "", IndexPath::new(0).section(1), 0) {
            Some(Entry::Chat(chat)) => assert_eq!(chat.title.as_ref(), "Second"),
            _ => panic!("expected the newest chat"),
        }
        // A query narrows both groups; the path resolves against the
        // filtered list.
        match entry_at(&chats, "second", IndexPath::new(0).section(1), 0) {
            Some(Entry::Chat(chat)) => assert_eq!(chat.title.as_ref(), "Second"),
            _ => panic!("expected the fuzzy-matched chat"),
        }
    });
}
