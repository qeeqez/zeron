//! In-app log capture: a `log`-facade logger that mirrors every record into
//! a bounded in-memory ring buffer (the View Logs panel reads it) plus a
//! rolling log file under `~/.rixl/rixlcode/logs/`. `install` runs once at
//! startup; tests push records straight into the buffer via `push`.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::LazyLock;

use log::{Level, Record};
use parking_lot::Mutex;

/// Ring-buffer capacity — enough history for a debugging session without
/// letting a chatty dependency grow memory unbounded.
pub(crate) const CAPACITY: usize = 2000;

/// Rotate the log file once it passes this size so it can't grow forever.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// One captured log line. `at` is preformatted (`YYYY-MM-DD HH:MM:SS.mmm`)
/// so the panel and Copy share one rendering with the file.
#[derive(Clone)]
pub(crate) struct LogRecord {
    pub at: String,
    pub level: Level,
    pub target: String,
    pub message: String,
}

impl LogRecord {
    fn new(record: &Record) -> Self {
        Self {
            at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            level: record.level(),
            target: record.target().to_string(),
            message: record.args().to_string(),
        }
    }

    /// The single-line rendering used by the panel, Copy, and the log file.
    pub(crate) fn line(&self) -> String {
        format!("{} {:<5} {}: {}", self.at, self.level, self.target, self.message)
    }
}

/// The panel's level filter — a minimum severity (`Warn` keeps warn+error).
/// `log::Level` orders most-severe-first, so `level <= min` is "at least".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogFilter {
    All,
    Info,
    Warn,
    Error,
}

impl LogFilter {
    pub(crate) const ALL: [LogFilter; 4] = [Self::All, Self::Info, Self::Warn, Self::Error];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Info => "Info+",
            Self::Warn => "Warn+",
            Self::Error => "Error",
        }
    }

    pub(crate) fn allows(self, level: Level) -> bool {
        match self {
            Self::All => true,
            Self::Info => level <= Level::Info,
            Self::Warn => level <= Level::Warn,
            Self::Error => level <= Level::Error,
        }
    }
}

/// The shared sink: the ring buffer the panel reads plus the open log file.
/// Lives behind a `LazyLock` so `push` works before `install` (tests) and
/// the `log` logger — a `&'static` — can reach it.
#[derive(Default)]
struct Sink {
    records: VecDeque<LogRecord>,
    file: Option<std::fs::File>,
}

static SINK: LazyLock<Mutex<Sink>> = LazyLock::new(|| Mutex::new(Sink::default()));

fn sink() -> &'static Mutex<Sink> {
    &SINK
}

/// Append a record to the ring buffer, evicting the oldest at capacity.
/// Tests drive the panel through this — no logger install needed.
pub(crate) fn push(record: LogRecord) {
    let mut sink = sink().lock();
    if sink.records.len() >= CAPACITY {
        sink.records.pop_front();
    }
    sink.records.push_back(record);
}

/// A record with the current timestamp — the shape `log` records produce.
#[cfg(test)]
pub(crate) fn record(level: Level, target: &str, message: &str) -> LogRecord {
    LogRecord {
        at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        level,
        target: target.to_string(),
        message: message.to_string(),
    }
}

/// A snapshot of the buffered records, oldest first.
pub(crate) fn records() -> Vec<LogRecord> {
    sink().lock().records.iter().cloned().collect()
}

/// The buffered lines passing `filter` — what the panel shows and Copy
/// writes to the clipboard.
pub(crate) fn filtered_lines(filter: LogFilter) -> Vec<String> {
    sink().lock().records.iter().filter(|r| filter.allows(r.level)).map(LogRecord::line).collect()
}

#[cfg(test)]
pub(crate) fn clear() {
    sink().lock().records.clear();
}

/// The log file path — `~/.rixl/rixlcode/logs/rixlcode.log`, beside the
/// per-project stores (see `crate::project::projects_dir`).
pub(crate) fn log_file_path() -> PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode/logs/rixlcode.log")
}

/// Install the capture logger. Called once from `main` before the app runs
/// so early startup records land in the buffer too. A second call (or a
/// logger installed by a dependency) is a no-op — `set_logger` fails and we
/// keep the buffer usable regardless.
pub(crate) fn install() {
    if let Some(file) = open_log_file() {
        sink().lock().file = Some(file);
    }
    if log::set_logger(&RingLogger).is_ok() {
        // Debug keeps a debug build's own chatter visible without letting
        // trace-level dependency spam drown the buffer.
        log::set_max_level(log::LevelFilter::Debug);
    }
}

/// Open the log file for appending, rotating the previous run's log aside
/// once it passes `MAX_FILE_BYTES`. Failures (read-only home, etc.) just
/// mean no file — the ring buffer still works.
fn open_log_file() -> Option<std::fs::File> {
    let path = log_file_path();
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

struct RingLogger;

impl log::Log for RingLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let entry = LogRecord::new(record);
        let line = entry.line();
        push(entry);
        if let Some(file) = sink().lock().file.as_mut() {
            let _ = writeln!(file, "{line}");
        }
    }

    fn flush(&self) {
        if let Some(file) = sink().lock().file.as_mut() {
            let _ = file.flush();
        }
    }
}
