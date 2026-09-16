//! Terminal tab tests: multiple PTY sessions in the panel — switching
//! tabs keeps background output, `x` kills only its own session, `+` is
//! capped, and every session survives the panel closing. Sessions come
//! from `TermSession::from_parts` with a fake PTY, so nothing real
//! spawns. Narrow imports — `use gpui_kit::*` would shadow `#[test]`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::composer_testutil::{open_workspace, until};
use crate::terminal::{Pty, PtyEvent, TermSession};
use crate::views::terminal::MAX_TERMINAL_TABS;
use crate::workspace::Workspace;

/// A PTY that only reports when it was dropped — closing a tab must kill
/// its own session and no other.
struct FlagPty {
    dropped: Arc<AtomicBool>,
}

impl Pty for FlagPty {
    fn write(&mut self, _bytes: &[u8]) {}
    fn resize(&mut self, _rows: u16, _cols: u16) {}
}

impl Drop for FlagPty {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// Push a fake session onto the panel; returns its output sender and the
/// flag that trips when the session (and its PTY) is dropped.
fn push_session(ws: &mut Workspace) -> (Sender<PtyEvent>, Arc<AtomicBool>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    ws.terminal
        .sessions
        .push(TermSession::from_parts(Box::new(FlagPty { dropped: dropped.clone() }), rx, 24, 80));
    (tx, dropped)
}

fn push_sessions(ws: &Entity<Workspace>, cx: &mut VisualTestContext, n: usize) -> Vec<(Sender<PtyEvent>, Arc<AtomicBool>)> {
    cx.update(|_, cx| ws.update(cx, |ws, _| (0..n).map(|_| push_session(ws)).collect()))
}

#[test]
fn switching_tabs_keeps_background_output() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let txs = push_sessions(&ws, cx, 2);
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.select_terminal_tab(0, cx);
            ws.toggle_terminal(window, cx);
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("terminal-tab", 0usize)).visible(), "tab 1 mounted");
        assert!(window.find(("terminal-tab", 1usize)).visible(), "tab 2 mounted");
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.terminal.active), 0);

    // Output lands on tab 1 while tab 2 is on screen: the pump still
    // drains it, but the panel renders only the active session.
    ws.update(cx, |ws, cx| ws.select_terminal_tab(1, cx));
    txs[0].0.send(PtyEvent::Output(b"bg-marker".to_vec())).unwrap();
    until(&ws, cx, |ws| ws.terminal.sessions[0].contents().contains("bg-marker"));
    let active = ws.read_with(cx, |ws, _| ws.terminal.active_session().unwrap().contents());
    assert!(!active.contains("bg-marker"), "background output rendered: {active:?}");

    // Switching back shows the retained scrollback.
    ws.update(cx, |ws, cx| ws.select_terminal_tab(0, cx));
    let active = ws.read_with(cx, |ws, _| ws.terminal.active_session().unwrap().contents());
    assert!(active.contains("bg-marker"), "output lost on switch: {active:?}");
}

#[test]
fn closing_a_tab_kills_only_that_session() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let txs = push_sessions(&ws, cx, 3);
    ws.update(cx, |ws, cx| ws.select_terminal_tab(2, cx));

    ws.update(cx, |ws, cx| ws.close_terminal_tab(2, cx));
    assert!(txs[2].1.load(Ordering::SeqCst), "closed session's PTY not dropped");
    assert!(!txs[0].1.load(Ordering::SeqCst) && !txs[1].1.load(Ordering::SeqCst), "other sessions killed");
    assert_eq!(ws.read_with(cx, |ws, _| (ws.terminal.sessions.len(), ws.terminal.active)), (2, 1));

    // Closing a tab before the active one shifts the index, not the session.
    ws.update(cx, |ws, cx| ws.close_terminal_tab(0, cx));
    assert_eq!(ws.read_with(cx, |ws, _| (ws.terminal.sessions.len(), ws.terminal.active)), (1, 0));
    assert!(!txs[1].1.load(Ordering::SeqCst), "remaining session killed");
}

#[test]
fn tab_cap_disables_new_tabs() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    push_sessions(&ws, cx, MAX_TERMINAL_TABS);
    ws.update(cx, |ws, cx| ws.new_terminal_tab(cx));
    assert_eq!(ws.read_with(cx, |ws, _| ws.terminal.sessions.len()), MAX_TERMINAL_TABS, "cap exceeded");

    // A freed slot takes a new tab again, selected on spawn.
    ws.update(cx, |ws, cx| {
        ws.close_terminal_tab(0, cx);
        ws.new_terminal_tab(cx);
    });
    assert_eq!(ws.read_with(cx, |ws, _| (ws.terminal.sessions.len(), ws.terminal.active)), (MAX_TERMINAL_TABS, MAX_TERMINAL_TABS - 1));
}

#[test]
fn sessions_survive_panel_close() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let txs = push_sessions(&ws, cx, 2);
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });

    // Close the panel, then emit output on both tabs — it must buffer in
    // the channels, not die with the pump.
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.try_find("terminal-panel").is_none(), "panel still mounted");
    });
    txs[0].0.send(PtyEvent::Output(b"while-closed-a".to_vec())).unwrap();
    txs[1].0.send(PtyEvent::Output(b"while-closed-b".to_vec())).unwrap();

    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    until(&ws, cx, |ws| {
        ws.terminal.sessions[0].contents().contains("while-closed-a") && ws.terminal.sessions[1].contents().contains("while-closed-b")
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.terminal.sessions.len()), 2);
}
