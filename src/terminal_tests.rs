//! Terminal tests: `TermSession` against a fake PTY (spawn spec, input,
//! scrollback, resize, exit) and the panel headless — open via the
//! titlebar button and Cmd-`, output lands in the scroll view, Enter in
//! the input line writes to the PTY, and open state persists. Narrow
//! imports — `use gpui_kit::*` would shadow `#[test]`.

use std::sync::mpsc::Sender;

use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, KeyBinding, TestAppContext, VisualTestContext};

use crate::composer_testutil::{open_workspace, until};
use crate::terminal::{Pty, PtyEvent, SpawnSpec, TermSession};
use crate::workspace::Workspace;
/// Shared write log the fake PTY records into.
type Writes = std::sync::Arc<parking_lot::Mutex<Vec<Vec<u8>>>>;
/// Shared resize log.
type Sizes = std::sync::Arc<parking_lot::Mutex<Vec<(u16, u16)>>>;

/// A scripted PTY: records writes/resizes, output arrives over a channel
/// the test controls.
struct FakePty {
    writes: Writes,
    sizes: Sizes,
}

impl Pty for FakePty {
    fn write(&mut self, bytes: &[u8]) {
        self.writes.lock().push(bytes.to_vec());
    }
    fn resize(&mut self, rows: u16, cols: u16) {
        self.sizes.lock().push((rows, cols));
    }
}

fn fake() -> (FakePty, Writes, Sizes) {
    let writes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sizes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    (FakePty { writes: writes.clone(), sizes: sizes.clone() }, writes, sizes)
}

#[test]
fn spawn_writes_the_shell_command() {
    let (pty, _, _) = fake();
    let (_tx, rx) = std::sync::mpsc::channel();
    let spec = SpawnSpec {
        program: "/bin/zsh".into(),
        args: vec!["-l".into()],
        cwd: std::path::PathBuf::from("/tmp/project"),
        rows: 24,
        cols: 80,
    };
    let session = TermSession::spawn(&spec, move |s| {
        assert_eq!(s.program, "/bin/zsh");
        assert_eq!(s.args, ["-l"]);
        assert_eq!(s.cwd, std::path::PathBuf::from("/tmp/project"));
        assert_eq!((s.rows, s.cols), (24, 80));
        (Box::new(pty), rx)
    });
    assert!(!session.exited);
}

#[test]
fn input_writes_to_the_pty() {
    let (pty, writes, _) = fake();
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut session = TermSession::from_parts(Box::new(pty), rx, 24, 80);
    session.write(b"ls\r");
    assert_eq!(writes.lock().as_slice(), [b"ls\r".to_vec()]);
}

#[test]
fn output_appends_to_the_scrollback() {
    let (pty, _, _) = fake();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut session = TermSession::from_parts(Box::new(pty), rx, 24, 80);
    tx.send(PtyEvent::Output(b"hello ".to_vec())).unwrap();
    tx.send(PtyEvent::Output(b"world\r\n$ ".to_vec())).unwrap();
    assert!(session.drain());
    let contents = session.contents();
    assert!(contents.contains("hello world"), "got: {contents:?}");
    assert!(contents.contains("$ "), "got: {contents:?}");
}

#[test]
fn ansi_output_is_parsed_not_raw() {
    let (pty, _, _) = fake();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut session = TermSession::from_parts(Box::new(pty), rx, 24, 80);
    tx.send(PtyEvent::Output(b"\x1b[31mred\x1b[0m".to_vec())).unwrap();
    assert!(session.drain());
    let contents = session.contents();
    assert!(contents.contains("red"), "got: {contents:?}");
    assert!(!contents.contains('\x1b'), "escape leaked: {contents:?}");
}

#[test]
fn resize_forwards_to_the_pty() {
    let (pty, _, sizes) = fake();
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut session = TermSession::from_parts(Box::new(pty), rx, 24, 80);
    session.resize(30, 120);
    assert_eq!(sizes.lock().as_slice(), [(30, 120)]);
    // Same size again is a no-op — no redundant SIGWINCH.
    session.resize(30, 120);
    assert_eq!(sizes.lock().len(), 1);
}

#[test]
fn exit_latches_the_session() {
    let (pty, _, _) = fake();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut session = TermSession::from_parts(Box::new(pty), rx, 24, 80);
    tx.send(PtyEvent::Output(b"bye".to_vec())).unwrap();
    tx.send(PtyEvent::Exited).unwrap();
    assert!(!session.drain());
    assert!(session.exited);
    assert!(session.contents().contains("bye"));
}

/// Install a fake PTY on the workspace's panel; returns the output sender
/// plus the recorded writes.
fn install_fake(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> (Sender<PtyEvent>, Writes) {
    let (tx, rx) = std::sync::mpsc::channel();
    let writes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let fake_writes = writes.clone();
    cx.update(|_, cx| {
        ws.update(cx, |ws, _| {
            ws.terminal.spawner = Some(Box::new(move |_spec| {
                (
                    Box::new(FakePty {
                        writes: fake_writes,
                        sizes: std::sync::Arc::new(parking_lot::Mutex::new(Vec::new())),
                    }),
                    rx,
                )
            }));
        });
    });
    (tx, writes)
}

#[test]
fn panel_opens_shows_output_and_sends_input() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let (tx, writes) = install_fake(&ws, cx);

    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("terminal-panel").visible(), "panel should be mounted");
    });
    assert!(ws.read_with(cx, |ws, _| ws.terminal.session.is_some()), "shell spawned on open");

    tx.send(PtyEvent::Output(b"fake-shell$ ".to_vec())).unwrap();
    until(&ws, cx, |ws| ws.terminal.session.as_ref().is_some_and(|s| s.contents().contains("fake-shell$")));
    cx.update(|window, _cx| {
        assert!(window.find("terminal-scroll").visible(), "scrollback view mounted");
    });

    // Type into the input line and press Enter — the text plus CR lands
    // on the PTY.
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.terminal.input.update(cx, |s: &mut InputState, cx| s.focus(window, cx));
        });
        window.input("echo hi", cx);
        window.press("enter", cx);
    });
    assert_eq!(writes.lock().as_slice(), [b"echo hi".to_vec(), b"\r".to_vec()]);
    cx.update(|window, cx| {
        assert!(ws.read_with(cx, |ws, app| ws.terminal.input.read(app).value().is_empty()), "input cleared");
        let _ = window;
    });
}

#[test]
fn cmd_backtick_toggles_the_panel() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    install_fake(&ws, cx);
    cx.update(|_, cx| {
        cx.bind_keys([KeyBinding::new("cmd-`", crate::ToggleTerminal, Some("workspace"))]);
    });
    cx.update(|window, cx| {
        window.press("cmd-`", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("terminal-panel").visible(), "cmd-` opens the panel");
        window.press("cmd-`", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("terminal-panel").is_none(), "cmd-` again closes it");
    });
}

#[test]
fn toggle_persists_open_state() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    install_fake(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    assert!(crate::persist::load_settings().terminal_open, "open persisted");
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    assert!(!crate::persist::load_settings().terminal_open, "closed persisted");
}
