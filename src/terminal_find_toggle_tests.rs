//! Terminal find bar's Match Case / Whole Word chips: `find_in_lines`
//! under each flag combo as pure scans, then the real bar headless —
//! chips narrow the match count live and stay set across close/reopen.
//! Declared from `views::terminal::find` via `#[path]` — `main.rs` is at
//! the SLOC cap. Narrow imports — `use gpui_kit::*` would shadow `#[test]`.

use std::sync::mpsc::Sender;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::chat_search::find_opts::FindOpts;
use crate::composer_testutil::{open_workspace, until};
use crate::terminal::links::find_in_lines;
use crate::terminal::{Pty, PtyEvent, TermSession};
use crate::workspace::Workspace;

const CASE: FindOpts = FindOpts { case_sensitive: true, whole_word: false };
const WORD: FindOpts = FindOpts { case_sensitive: false, whole_word: true };
const BOTH: FindOpts = FindOpts { case_sensitive: true, whole_word: true };

/// A PTY that swallows input — output arrives over the channel the test
/// controls.
struct FakePty;

impl Pty for FakePty {
    fn write(&mut self, _bytes: &[u8]) {}
    fn resize(&mut self, _rows: u16, _cols: u16) {}
}

/// Push a scripted session onto the panel and make it active; returns the
/// sender so more output can follow.
fn push_session(ws: &Entity<Workspace>, cx: &mut VisualTestContext, output: &[u8]) -> Sender<PtyEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(PtyEvent::Output(output.to_vec())).unwrap();
    cx.update(|_, cx| {
        ws.update(cx, |ws, _| {
            ws.terminal.sessions.push(TermSession::from_parts(Box::new(FakePty), rx, 24, 80));
            ws.terminal.active = ws.terminal.sessions.len() - 1;
        });
    });
    tx
}

#[test]
fn match_case_drops_folded_lookalikes() {
    let contents = "Hit me\nhit me\nHIT";
    assert_eq!(find_in_lines(contents, "hit", FindOpts::default()).len(), 3, "default folds case");
    let matches = find_in_lines(contents, "hit", CASE);
    assert_eq!(matches.len(), 1, "exact case only");
    assert_eq!(matches[0].line, 1);
    assert_eq!(&contents[matches[0].range.clone()], "hit");
}

#[test]
fn whole_word_requires_non_word_flanks() {
    let contents = "hitter hit a_hit hit-or-miss";
    assert_eq!(find_in_lines(contents, "hit", FindOpts::default()).len(), 4);
    let matches = find_in_lines(contents, "hit", WORD);
    assert_eq!(matches.len(), 2, "'hitter' and 'a_hit' are word-flanked; '-' isn't");
    assert_eq!(&contents[matches[0].range.clone()], "hit");
    assert_eq!(&contents[matches[1].range.clone()], "hit");
}

#[test]
fn toggles_combine_and_stay_line_scoped() {
    let contents = "Hit\nhit\nHitter";
    let lower = find_in_lines(contents, "hit", BOTH);
    assert_eq!(lower.len(), 1, "only the lowercase whole word survives — 'Hit' fails case, 'Hitter' fails the boundary");
    assert_eq!(lower[0].line, 1);
    let matches = find_in_lines(contents, "Hit", BOTH);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line, 0);
    assert_eq!(&contents[matches[0].range.clone()], "Hit");
}

#[test]
fn toggle_chips_narrow_matches_and_stick() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push_session(&ws, cx, b"Hit me\nhit me\nhitter\r\n");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    until(&ws, cx, |ws| ws.terminal.active_session().is_some_and(|s| s.contents().contains("hitter")));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("terminal-find").visible(), "cmd-f opens the terminal find bar");
    });
    cx.update(|window, cx| {
        window.input("hit", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 3"), "insensitive substring matches all three");
        assert_eq!(window.find("term-find-match-case").checked(), Some(false), "Match Case starts off");
        assert_eq!(window.find("term-find-whole-word").checked(), Some(false), "Whole Word starts off");
    });
    // Chip clicks dispatch real mouse events at the element bounds — let
    // the bar settle first so the elements aren't mid-layout.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("term-find-match-case", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("term-find-match-case").checked(), Some(true));
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 2"), "'Hit me' loses the mark");
        window.click("term-find-whole-word", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 1"), "'hitter' is word-flanked");
        assert!(ws.read(cx).terminal.find.opts.case_sensitive && ws.read(cx).terminal.find.opts.whole_word);
        window.click("term-find-match-case", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 2"), "case off: 'Hit me' returns");
        // Chips stay set for the session — the same stickiness the chat
        // find bar's toggles have.
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("terminal-find").is_none(), "esc closes the bar");
    });
    // The deferred focus back to the terminal input lands between updates;
    // cmd-f must wait for it or the keystroke leaves the panel's scope.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("terminal-find").visible(), "cmd-f reopens the bar");
        assert_eq!(window.find("term-find-whole-word").checked(), Some(true), "Whole Word stays set");
        assert_eq!(window.find("term-find-match-case").checked(), Some(false));
    });
}
