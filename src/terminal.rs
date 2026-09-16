//! The embedded terminal's PTY session: spawn the user's shell in the
//! project dir, stream its output into a `vt100` screen (scrollback +
//! ANSI-aware rendering), forward input and resizes. The reader runs on a
//! background thread — the UI only ever drains a channel, so a chatty
//! process can't stall a frame.
//!
//! `Pty` is the seam tests use: `TermSession::from_parts` accepts any
//! implementation plus a scripted event stream, so no test spawns a real
//! process.
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// Screen-text scans for the panel's find bar and clickable spans —
/// `#[path]` keeps `main.rs` under the SLOC cap.
#[path = "terminal_links.rs"]
pub(crate) mod links;

/// Bytes the shell produced, or its exit. `Exited` is sent exactly once,
/// when the reader hits EOF or an error.
pub(crate) enum PtyEvent {
    Output(Vec<u8>),
    Exited,
}

/// What a session needs to spawn a shell: the command line, the working
/// directory and the initial terminal size.
#[derive(Debug)]
pub(crate) struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
}

/// The writable/resizable side of a PTY. Output arrives separately via the
/// `PtyEvent` channel a session is built with.
pub(crate) trait Pty: Send {
    fn write(&mut self, bytes: &[u8]);
    fn resize(&mut self, rows: u16, cols: u16);
}

/// Scrollback kept by the vt100 parser, in rows beyond the visible screen.
const SCROLLBACK: usize = 2000;

/// One live terminal: the PTY handle, its output stream and the parsed
/// screen state. `exited` latches when the shell dies — the panel keeps
/// showing the final screen.
pub(crate) struct TermSession {
    pty: Box<dyn Pty>,
    events: Receiver<PtyEvent>,
    screen: vt100::Parser,
    pub exited: bool,
}

impl TermSession {
    /// Spawn through `spawn` (`spawn_native` in the app, a fake in tests)
    /// and start the reader thread.
    pub(crate) fn spawn(spec: &SpawnSpec, spawn: impl FnOnce(&SpawnSpec) -> (Box<dyn Pty>, Receiver<PtyEvent>)) -> Self {
        let (pty, events) = spawn(spec);
        Self::from_parts(pty, events, spec.rows, spec.cols)
    }

    /// A session from an already-running PTY and its event stream — the
    /// test seam: a fake Pty plus a channel the test feeds.
    pub(crate) fn from_parts(pty: Box<dyn Pty>, events: Receiver<PtyEvent>, rows: u16, cols: u16) -> Self {
        Self {
            pty,
            events,
            screen: vt100::Parser::new(rows, cols, SCROLLBACK),
            exited: false,
        }
    }

    /// The screen's text — scrollback plus the visible grid, ANSI already
    /// resolved by the parser.
    pub(crate) fn contents(&self) -> String {
        self.screen.screen().contents()
    }

    /// The visible grid size — `(rows, cols)`. The link scan covers only
    /// these lines of the rendered contents, never the scrollback.
    pub(crate) fn size(&self) -> (u16, u16) {
        self.screen.screen().size()
    }

    /// Forward one input line to the shell.
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        self.pty.write(bytes);
    }

    /// Forward a size change to both the PTY (SIGWINCH for the child) and
    /// the parser (re-wrap the screen).
    pub(crate) fn resize(&mut self, rows: u16, cols: u16) {
        let size = self.screen.screen().size();
        if size == (rows, cols) {
            return;
        }
        self.pty.resize(rows, cols);
        self.screen.screen_mut().set_size(rows, cols);
    }

    /// Drain pending output into the screen. Returns `false` once the
    /// session has ended — the caller stops polling.
    pub(crate) fn drain(&mut self) -> bool {
        loop {
            match self.events.try_recv() {
                Ok(PtyEvent::Output(bytes)) => self.screen.process(&bytes),
                Ok(PtyEvent::Exited) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.exited = true;
                    return false;
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => return !self.exited,
            }
        }
    }
}

/// The user's login shell: `$SHELL` when set, zsh otherwise (the macOS
/// default). `-l` makes it a login shell so PATH and friends load like a
/// real terminal.
pub(crate) fn shell_spec() -> (String, Vec<String>) {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".to_string());
    (shell, vec!["-l".to_string()])
}

/// Spawn `spec`'s command on a real PTY via `portable-pty` and return the
/// handle plus its event stream. A spawn failure still yields a session —
/// the error is written into the scrollback so the panel shows it.
pub(crate) fn spawn_native(spec: &SpawnSpec) -> (Box<dyn Pty>, Receiver<PtyEvent>) {
    let (tx, rx) = std::sync::mpsc::channel();
    match try_spawn_native(spec, tx.clone()) {
        Ok(pty) => (Box::new(pty), rx),
        Err(err) => {
            let _ = tx.send(PtyEvent::Output(format!("terminal: {err}\r\n").into_bytes()));
            let _ = tx.send(PtyEvent::Exited);
            (Box::new(NullPty), rx)
        },
    }
}

/// A dead-end PTY for a failed spawn — writes and resizes go nowhere.
struct NullPty;

impl Pty for NullPty {
    fn write(&mut self, _bytes: &[u8]) {}
    fn resize(&mut self, _rows: u16, _cols: u16) {}
}

/// The real PTY: master end for resize, writer for input, child handle so
/// drop can kill + reap the shell off the UI thread.
struct NativePty {
    master: Box<dyn portable_pty::MasterPty>,
    writer: Box<dyn Write + Send>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
}

impl Pty for NativePty {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        let _ = self.master.resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }
}

impl Drop for NativePty {
    /// Kill the shell and reap it on a helper thread — `wait` can block
    /// briefly and `Drop` may run on the UI thread.
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

fn try_spawn_native(spec: &SpawnSpec, tx: Sender<PtyEvent>) -> Result<NativePty, Box<dyn std::error::Error + Send + Sync>> {
    let system = portable_pty::native_pty_system();
    let pair = system.openpty(portable_pty::PtySize {
        rows: spec.rows,
        cols: spec.cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = portable_pty::CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    cmd.cwd(&spec.cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let child = pair.slave.spawn_command(cmd)?;
    // The slave end belongs to the child — dropping ours lets the master
    // see EOF when the shell exits.
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    std::thread::spawn(move || read_loop(&mut *reader, &tx));
    Ok(NativePty { master: pair.master, writer, child: Some(child) })
}

/// Pump PTY output into the event channel until EOF or error. Runs on its
/// own thread so a blocking read never touches the UI.
fn read_loop(reader: &mut dyn Read, tx: &Sender<PtyEvent>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                    return;
                }
            },
        }
    }
    let _ = tx.send(PtyEvent::Exited);
}
