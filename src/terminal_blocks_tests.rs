//! Command-block tests: block boundaries over a mocked session (fake
//! PTY + scripted output, nothing real spawns) — submit records a
//! block, output splits between blocks, scrollback keeps boundaries
//! stable, OSC 133 marks pin the output start and exit code, re-run
//! resubmits, and exited sessions go read-only. Narrow imports —
//! `use gpui_kit::*` would shadow `#[test]`.

use std::sync::mpsc::Sender;

use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::composer_testutil::{composer_value, open_workspace, until};
use crate::terminal::{Pty, PtyEvent, TermSession};
use crate::views::terminal_blocks::block_quote;
use crate::workspace::Workspace;

/// Shared write log the fake PTY records into — same shape as
/// `terminal_tests`' `Writes`.
type Writes = std::sync::Arc<parking_lot::Mutex<Vec<Vec<u8>>>>;
struct FakePty {
    writes: Writes,
}
impl Pty for FakePty {
    fn write(&mut self, bytes: &[u8]) {
        self.writes.lock().push(bytes.to_vec());
    }

    fn resize(&mut self, _rows: u16, _cols: u16) {}
}

/// A session on a fake PTY; returns it, its output sender, and the
/// recorded writes.
fn session(rows: u16, cols: u16) -> (TermSession, Sender<PtyEvent>, Writes) {
    let (tx, rx) = std::sync::mpsc::channel();
    let writes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let pty = FakePty { writes: writes.clone() };
    (TermSession::from_parts(Box::new(pty), rx, rows, cols), tx, writes)
}

/// Feed output and drain — the pump's job, inline for tests.
fn feed(session: &mut TermSession, tx: &Sender<PtyEvent>, bytes: &[u8]) {
    tx.send(PtyEvent::Output(bytes.to_vec())).unwrap();
    session.drain();
}

#[test]
fn submit_records_a_block() {
    let (mut s, tx, writes) = session(24, 80);
    feed(&mut s, &tx, b"$ ");
    s.submit_command("ls");
    assert_eq!(writes.lock().as_slice(), [b"ls".to_vec(), b"\r".to_vec()]);
    assert_eq!(s.blocks.len(), 1);
    assert_eq!(s.blocks[0].command, "ls");
    // The cursor sat on row 0 col 2 — the echo occupies that row, output
    // is estimated from the next one.
    assert_eq!(s.blocks[0].start, 0);
    assert_eq!(s.blocks[0].output, 1);
}

#[test]
fn empty_submit_opens_no_block() {
    let (mut s, tx, _) = session(24, 80);
    feed(&mut s, &tx, b"$ ");
    s.submit_command("");
    assert!(s.blocks.is_empty());
    s.submit_command("  ");
    assert_eq!(s.blocks.len(), 1, "whitespace still submits — the shell decides");
}

#[test]
fn blocks_split_output_between_commands() {
    let (mut s, tx, _) = session(24, 80);
    feed(&mut s, &tx, b"$ ");
    s.submit_command("ls");
    feed(&mut s, &tx, b"ls\r\nfile1\r\nfile2\r\n$ ");
    s.submit_command("pwd");
    feed(&mut s, &tx, b"pwd\r\n/tmp\r\n$ ");
    let t = s.transcript();
    let layout = s.block_layout(&t);
    assert_eq!(layout.len(), 2);
    // A block's output runs to the next command's echo row — the prompt
    // line it shares is hidden with that echo, not copied as output.
    assert_eq!(t.lines[layout[0].out.clone()].join("\n"), "file1\nfile2");
    assert_eq!(t.lines[layout[1].out.clone()].join("\n"), "/tmp\n$ ");
    // Each header hides exactly its echo row — prompt plus echoed text.
    assert_eq!(t.lines[layout[0].hide.clone()], ["$ ls"]);
    assert_eq!(t.lines[layout[1].hide.clone()], ["$ pwd"]);
    assert_eq!(s.block_output(0), "file1\nfile2");
}

#[test]
fn scrollback_keeps_block_boundaries() {
    let (mut s, tx, _) = session(4, 80);
    feed(&mut s, &tx, b"$ ");
    s.submit_command("seq");
    feed(&mut s, &tx, b"seq\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n$ ");
    s.submit_command("pwd");
    feed(&mut s, &tx, b"pwd\r\n/tmp\r\n$ ");
    let t = s.transcript();
    // The first lines scrolled into history — the transcript holds them.
    assert!(t.lines.len() > 4, "transcript exceeds the screen: {} lines", t.lines.len());
    assert_eq!(t.lines[0], "$ seq");
    let layout = s.block_layout(&t);
    assert_eq!(t.lines[layout[0].hide.clone()], ["$ seq"]);
    assert_eq!(t.lines[layout[1].hide.clone()], ["$ pwd"]);
    assert_eq!(s.block_output(0), "1\n2\n3\n4\n5\n6");
    assert_eq!(s.block_output(1), "/tmp\n$ ");
}

#[test]
fn osc133_marks_pin_output_and_exit() {
    let (mut s, tx, _) = session(24, 80);
    feed(&mut s, &tx, b"\x1b]133;A\x07$ ");
    s.submit_command("false");
    feed(&mut s, &tx, b"false\r\n\x1b]133;C\x07oops\r\n\x1b]133;D;1\x07\x1b]133;A\x07$ ");
    let t = s.transcript();
    let layout = s.block_layout(&t);
    assert_eq!(layout.len(), 1);
    assert_eq!(layout[0].exit, Some(1));
    // 133;C pinned the output start — the echo row is hidden, "oops" is
    // the whole output, and 133;D ends it before the next prompt.
    assert_eq!(s.block_output(0), "oops");
    assert_eq!(t.lines[layout[0].hide.clone()], ["$ false"]);
}

#[test]
fn rerun_submits_the_command_again() {
    let (mut s, tx, writes) = session(24, 80);
    feed(&mut s, &tx, b"$ ");
    s.submit_command("ls");
    feed(&mut s, &tx, b"ls\r\n$ ");
    s.rerun(0);
    assert_eq!(writes.lock().as_slice(), [b"ls".to_vec(), b"\r".to_vec(), b"ls".to_vec(), b"\r".to_vec()]);
    assert_eq!(s.blocks.len(), 2, "the re-run opens its own block");
    assert_eq!(s.blocks[1].command, "ls");
}

#[test]
fn exited_session_keeps_blocks_read_only() {
    let (mut s, tx, writes) = session(24, 80);
    feed(&mut s, &tx, b"$ ");
    s.submit_command("ls");
    feed(&mut s, &tx, b"ls\r\nfile\r\n$ ");
    tx.send(PtyEvent::Exited).unwrap();
    assert!(!s.drain());
    s.rerun(0);
    assert_eq!(writes.lock().len(), 2, "re-run writes nothing once exited");
    assert_eq!(s.blocks.len(), 1);
    // The block's output is still readable — read-only, not gone.
    assert_eq!(s.block_output(0), "file\n$ ");
}

#[test]
fn panel_renders_block_headers_and_reruns() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let (tx, rx) = std::sync::mpsc::channel();
    let writes: Writes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let fake_writes = writes.clone();
    cx.update(|_, cx| {
        ws.update(cx, |ws, _| {
            ws.terminal
                .spawners
                .push_back(Box::new(move |_spec| (Box::new(FakePty { writes: fake_writes }), rx)));
        });
    });
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    tx.send(PtyEvent::Output(b"$ ".to_vec())).unwrap();
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.terminal.input.update(cx, |s: &mut InputState, cx| s.focus(window, cx));
        });
        window.input("echo hi", cx);
        window.press("enter", cx);
    });
    tx.send(PtyEvent::Output(b"echo hi\r\nhi\r\n$ ".to_vec())).unwrap();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("term-block", 0usize)).visible(), "block header rendered");
        assert!(window.try_find(("term-rerun", 0usize)).is_some(), "re-run affordance mounted");
        window.hover(("term-block", 0usize), cx);
        window.draw(cx).clear(cx);
        window.click(("term-rerun", 0usize), cx);
    });
    assert_eq!(
        writes.lock().as_slice(),
        [b"echo hi".to_vec(), b"\r".to_vec(), b"echo hi".to_vec(), b"\r".to_vec()],
        "re-run writes the command back to the PTY"
    );
    assert_eq!(ws.read_with(cx, |ws, _| ws.terminal.sessions[0].blocks.len()), 2);
}

#[test]
fn block_quote_wraps_output_with_command_provenance() {
    assert_eq!(block_quote("ls", "a\nb"), "$ ls\n```text\na\nb\n```");
    // Trailing blanks drop, and a whitespace-only command loses its `$` line.
    assert_eq!(block_quote("  ", "out\n\n"), "```text\nout\n```");
}

#[test]
fn block_quote_empty_output_is_empty() {
    assert!(block_quote("ls", "").is_empty());
    assert!(block_quote("ls", "  \n \n").is_empty());
}

#[test]
fn block_quote_fence_outruns_backticks() {
    assert_eq!(block_quote("cat", "```\nfenced\n```"), "$ cat\n````text\n```\nfenced\n```\n````");
}

/// Mount a workspace with a fake-PTY terminal session, submit `cmd` via
/// the real input path, then feed `output` back as the shell's bytes.
fn panel_session(cx: &mut VisualTestContext, ws: &Entity<Workspace>, cmd: &str, output: &[u8]) -> Sender<PtyEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    let writes: Writes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let fake_writes = writes.clone();
    cx.update(|_, cx| {
        ws.update(cx, |ws, _| {
            ws.terminal
                .spawners
                .push_back(Box::new(move |_spec| (Box::new(FakePty { writes: fake_writes }), rx)));
        });
    });
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_terminal(window, cx));
    });
    tx.send(PtyEvent::Output(b"$ ".to_vec())).unwrap();
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.terminal.input.update(cx, |s: &mut InputState, cx| s.focus(window, cx));
        });
        window.input(cmd, cx);
        window.press("enter", cx);
    });
    tx.send(PtyEvent::Output(output.to_vec())).unwrap();
    tx
}

#[test]
fn panel_send_button_quotes_output_into_composer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    panel_session(cx, &ws, "echo hi", b"echo hi\r\nhi\r\n$ ");
    // The drain pump runs on a 50ms timer — wait until the output lands.
    until(&ws, cx, |ws| !ws.terminal.sessions[0].block_output(0).is_empty());
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.composer.update(cx, |s, cx| s.set_value("explain this", window, cx));
        });
        window.draw(cx).clear(cx);
        window.hover(("term-block", 0usize), cx);
        window.draw(cx).clear(cx);
        window.click(("term-send", 0usize), cx);
    });
    let value = composer_value(&ws, cx);
    assert!(value.starts_with("$ echo hi\n```text\nhi"), "quoted block leads the draft: {value:?}");
    assert!(value.ends_with("explain this"), "existing draft kept below the quote: {value:?}");
}

#[test]
fn panel_send_button_no_output_keeps_draft() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    // The command is still running — nothing echoed past its own line yet.
    panel_session(cx, &ws, "sleep 9", b"sleep 9\r\n");
    until(&ws, cx, |ws| ws.terminal.sessions[0].transcript().lines.iter().any(|l| l.contains("sleep 9")));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.composer.update(cx, |s, cx| s.set_value("keep me", window, cx));
        });
        window.draw(cx).clear(cx);
        window.hover(("term-block", 0usize), cx);
        window.draw(cx).clear(cx);
        window.click(("term-send", 0usize), cx);
    });
    assert_eq!(composer_value(&ws, cx), "keep me", "empty output leaves the draft alone");
}
