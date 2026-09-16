//! Terminal search + clickable-span tests: `find_in_lines` positions and
//! `detect_links` spans as pure scans, then the panel headless — Cmd-F
//! opens the bar over the terminal, Enter/Shift-Enter cycle matches, Esc
//! clears, a tab switch re-runs the query on the new session, and a
//! Cmd-click on a path lands an `@path ` mention in the composer.
//! Narrow imports — `use gpui_kit::*` would shadow `#[test]`.

use std::sync::mpsc::Sender;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::composer_testutil::{open_workspace, until};
use crate::terminal::links::{detect_links, find_in_lines};
use crate::terminal::{Pty, PtyEvent, TermSession};
use crate::workspace::Workspace;

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
fn find_reports_line_and_byte_range() {
    let contents = "alpha beta\nbeta BETA gamma\nno hit";
    let matches = find_in_lines(contents, "beta");
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].line, 0);
    assert_eq!(&contents[matches[0].range.clone()], "beta");
    assert_eq!(matches[1].line, 1);
    assert_eq!(&contents[matches[1].range.clone()], "beta");
    assert_eq!(matches[2].line, 1);
    assert_eq!(&contents[matches[2].range.clone()], "BETA", "case-insensitive");
    assert!(find_in_lines(contents, "").is_empty(), "empty query matches nothing");
    assert!(find_in_lines(contents, "zzz").is_empty());
}

#[test]
fn detect_links_finds_urls_and_real_paths() {
    let dir = std::env::temp_dir().join(format!("rixlcode-links-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/linked.rs"), b"fn main() {}\n").unwrap();
    let contents = "see https://example.com/docs. and src/linked.rs:9 plus gone/missing.rs";
    let links = detect_links(contents, 0..1, &dir, |p| p.exists());
    assert_eq!(links.len(), 2, "url + existing path only");
    assert!(links[0].is_url);
    assert_eq!(links[0].target, "https://example.com/docs", "trailing period trimmed");
    assert_eq!(&contents[links[0].range.clone()], "https://example.com/docs");
    assert!(!links[1].is_url);
    assert_eq!(links[1].target, "src/linked.rs", "project-relative target");
    assert_eq!(&contents[links[1].range.clone()], "src/linked.rs:9", "span covers :line");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn detect_links_scans_visible_rows_only() {
    let dir = std::env::temp_dir().join(format!("rixlcode-vis-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("real.txt"), b"x\n").unwrap();
    // Line 0 is "scrollback" — outside the visible range it is never
    // scanned, so its real path must not link.
    let contents = "real.txt in scrollback\nreal.txt on screen\n";
    let links = detect_links(contents, 1..3, &dir, |p| p.exists());
    assert_eq!(links.len(), 1);
    assert!(links[0].range.start > contents.find('\n').unwrap(), "only the visible line links");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cmd_f_searches_the_active_session() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push_session(&ws, cx, b"one hit\ntwo hit\nthree hit\r\n");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    until(&ws, cx, |ws| ws.terminal.active_session().is_some_and(|s| s.contents().contains("three hit")));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("terminal-find").visible(), "cmd-f opens the terminal find bar");
        assert!(ws.read(cx).terminal.find.open);
    });
    cx.update(|window, cx| {
        window.input("hit", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 3"));
        window.press("enter", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).terminal.find.match_ix, 1);
        assert_eq!(window.find("terminal-find-count").label(), Some("2 / 3"));
        window.press("shift-enter", cx);
        window.press("shift-enter", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).terminal.find.match_ix, 2, "shift-enter wraps back to the last match");
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("terminal-find").is_none(), "esc closes the bar");
        assert!(!ws.read(cx).terminal.find.open);
    });
    assert!(ws.read_with(cx, |ws, app| ws.terminal.find.input.read(app).value().is_empty()), "closing clears the query");
}

#[test]
fn switching_tabs_retargets_the_find_query() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push_session(&ws, cx, b"hit alpha\r\n");
    push_session(&ws, cx, b"hit beta hit gamma\r\n");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    until(&ws, cx, |ws| ws.terminal.active_session().is_some_and(|s| s.contents().contains("hit gamma")));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.press("cmd-f", cx);
    });
    cx.update(|window, cx| {
        window.input("hit", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 2"), "active tab has two hits");
        ws.update(cx, |ws, cx| ws.select_terminal_tab(0, cx));
        window.draw(cx).clear(cx);
        assert_eq!(window.find("terminal-find-count").label(), Some("1 / 1"), "query re-ran on the other session");
        assert!(ws.read(cx).terminal.find.open, "the bar stays open across the switch");
    });
}

#[test]
fn cmd_click_path_mentions_it_in_the_composer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    // `Cargo.toml` exists at the project root — the link scan resolves it
    // without touching the filesystem setup.
    push_session(&ws, cx, b"open Cargo.toml or https://example.com/x\r\n");
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    until(&ws, cx, |ws| ws.terminal.active_session().is_some_and(|s| s.contents().contains("Cargo.toml")));
    let links = cx.update(|_, cx| {
        ws.update(cx, |ws, _cx| {
            let contents = ws.terminal.active_session().unwrap().contents();
            ws.term_links(&contents)
        })
    });
    assert_eq!(links.len(), 2, "path + url detected on the visible screen");
    let path_link = links.iter().find(|l| !l.is_url).unwrap();
    assert_eq!(path_link.target, "Cargo.toml");
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.term_link_click(path_link, window, cx));
    });
    assert_eq!(
        ws.read_with(cx, |ws, app| ws.composer.read(app).value().to_string()),
        "@Cargo.toml ",
        "cmd-click on a path inserts the composer mention"
    );
    let url_link = links.iter().find(|l| l.is_url).unwrap().clone();
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.term_link_click(&url_link, window, cx));
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/x"), "cmd-click on a url opens it");
}
