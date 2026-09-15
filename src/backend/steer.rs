//! Mid-turn steering: the `TurnHandle` a `ReplyStream` hands the UI, the
//! shared stdin a steer writes through, and the `turn/steer` wire request.
//!
//! Codex's app-server accepts `turn/steer` on the live connection — the
//! running turn picks the text up as new user input without a restart.
//! Backends without mid-turn input get the default handle (a bare child
//! slot): `steer` declines and the caller queues the message instead.

use std::io::Write;
use std::sync::atomic::{AtomicI64, Ordering};

use parking_lot::Mutex;
use serde_json::{Value, json};

/// Live handle for one in-flight turn — `ReplyStream::child` / the chat's
/// stream slot. Killing the turn is universal; steering is codex-only.
pub trait TurnHandle: Send + Sync {
    /// Kill and reap the turn's process, if any.
    fn kill(&self);
    /// Inject `text` into the running turn. False when the turn can't take
    /// input right now (no live stdin, handshake incomplete) — the caller
    /// falls back to queueing.
    fn steer(&self, _text: &str) -> bool {
        false
    }
}

/// The default handle: a bare child slot. Kills work; steering doesn't.
impl TurnHandle for Mutex<Option<std::process::Child>> {
    fn kill(&self) {
        if let Some(mut c) = self.lock().take() {
            let _ = c.kill();
            // Reap off-thread — a child in uninterruptible sleep would
            // block the UI on wait().
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }
}

/// Shared handles delegate — `Arc<Mutex<…>>` slots and `Arc<CodexSlot>`
/// both satisfy `&dyn TurnHandle` at call sites.
impl<T: TurnHandle + ?Sized> TurnHandle for std::sync::Arc<T> {
    fn kill(&self) {
        (**self).kill();
    }
    fn steer(&self, text: &str) -> bool {
        (**self).steer(text)
    }
}

/// Kill and reap the turn's child, if any.
pub(crate) fn kill_slot(slot: &dyn TurnHandle) {
    slot.kill();
}

/// The child process's stdin, shared between the pump thread (handshake,
/// server-request replies) and the UI's steer calls. `None` once the turn
/// ends so late steers fail fast instead of hitting EPIPE.
#[derive(Clone)]
pub(super) struct SharedStdin(pub(super) std::sync::Arc<Mutex<Option<Box<dyn Write + Send>>>>);

impl SharedStdin {
    pub(super) fn new() -> Self {
        Self(std::sync::Arc::new(Mutex::new(None)))
    }

    /// Write one NDJSON line atomically — the lock is held across the whole
    /// line so a steer can't interleave with a server-request reply.
    pub(super) fn write_line(&self, v: &Value) -> Result<(), String> {
        let mut line = v.to_string();
        line.push('\n');
        self.lock_and(|w| w.write_all(line.as_bytes()).and_then(|()| w.flush()))
    }

    fn lock_and(&self, f: impl FnOnce(&mut dyn Write) -> std::io::Result<()>) -> Result<(), String> {
        let mut guard = self.0.lock();
        let Some(w) = guard.as_mut() else { return Err("codex stdin closed".into()) };
        f(w).map_err(|e| format!("codex stdin: {e}"))
    }
}

/// `SharedStdin` also satisfies `Write` so the handshake's `&mut dyn Write`
/// plumbing works unchanged. Per-call writes may interleave with a steer —
/// only used before the turn id exists, when no steer can fire.
impl Write for SharedStdin {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lock_and(|w| w.write(buf).map(|_| ())).map_err(std::io::Error::other)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.lock_and(|w| w.flush()).map_err(std::io::Error::other)
    }

    /// Format the whole line under one lock — `writeln!` on a `dyn Write`
    /// would otherwise fragment and let a steer interleave mid-line.
    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        self.lock_and(|w| w.write_all(args.to_string().as_bytes())).map_err(std::io::Error::other)
    }
}

/// `turn/steer`: inject user text into the running turn. `expected_turn_id`
/// is the server's precondition — a stale turn id rejects the request
/// instead of steering the wrong turn.
pub(super) fn turn_steer_req(id: i64, thread_id: &str, turn_id: &str, text: &str) -> Value {
    json!({
        "method": "turn/steer",
        "id": id,
        "params": {
            "threadId": thread_id,
            "expectedTurnId": turn_id,
            "input": [{"type": "text", "text": text, "text_elements": []}],
        },
    })
}

/// A `Write` that appends into a shared buffer — lets steer tests read
/// back what a turn's stdin received without spawning a process.
#[cfg(test)]
pub(super) struct SharedBuf(pub std::sync::Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
impl SharedStdin {
    /// A stdin that captures writes into `buf`.
    pub(super) fn for_test(buf: std::sync::Arc<Mutex<Vec<u8>>>) -> Self {
        let s = Self::new();
        *s.0.lock() = Some(Box::new(SharedBuf(buf)));
        s
    }
}

#[cfg(test)]
impl CodexSlot {
    /// A slot whose stdin captures writes into `buf`.
    pub(super) fn for_test(buf: std::sync::Arc<Mutex<Vec<u8>>>) -> Self {
        Self { stdin: SharedStdin::for_test(buf), ..Self::new() }
    }
}

/// Codex's turn handle: the child slot plus everything `turn/steer` needs —
/// the shared stdin and the thread/turn ids the handshake records.
pub(super) struct CodexSlot {
    pub child: Mutex<Option<std::process::Child>>,
    /// Live stdin for the current attempt — replaced on each respawn.
    pub stdin: SharedStdin,
    /// `(thread_id, turn_id)` once `thread/start` and `turn/start` answer.
    /// Steer needs both; `None` until the handshake reaches that phase.
    pub ids: Mutex<(Option<String>, Option<String>)>,
    /// Request ids for steers — handshake owns 1–3, steers count up from 4.
    next_id: AtomicI64,
}

impl CodexSlot {
    pub(super) fn new() -> Self {
        Self {
            child: Mutex::new(None),
            stdin: SharedStdin::new(),
            ids: Mutex::new((None, None)),
            next_id: AtomicI64::new(4),
        }
    }
}

impl TurnHandle for CodexSlot {
    fn kill(&self) {
        self.child.kill();
    }

    fn steer(&self, text: &str) -> bool {
        // Snapshot ids first, then take the stdin lock — the pump holds
        // stdin while writing and touches ids unlocked, so this order can
        // never deadlock against it.
        let (Some(thread_id), Some(turn_id)) = self.ids.lock().clone() else { return false };
        let req = turn_steer_req(self.next_id.fetch_add(1, Ordering::Relaxed), &thread_id, &turn_id, text);
        self.stdin.write_line(&req).is_ok()
    }
}
