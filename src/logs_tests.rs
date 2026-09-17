//! Tests for the log capture (`crate::logs`) and the View Logs panel:
//! records land in the ring buffer, the buffer evicts at capacity, the
//! level filter selects by severity, Copy writes the visible lines, and
//! Cmd-Shift-L opens the panel headlessly.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, point, px};

use crate::composer_testutil::open_workspace;
use crate::logs::{self, LogFilter};

/// The buffer is process-global — clear it so each test sees only its own
/// records (nextest gives every test its own process, but a test still
/// shares the buffer with anything the workspace logged during setup).
fn reset() {
    logs::clear();
}

#[test]
fn records_land_in_buffer() {
    reset();
    logs::push(logs::record(log::Level::Info, "rixlcode::backend", "turn started"));
    logs::push(logs::record(log::Level::Error, "rixlcode::send", "reply failed"));

    let records = logs::records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].level, log::Level::Info);
    assert_eq!(records[0].target, "rixlcode::backend");
    assert_eq!(records[0].message, "turn started");
    assert_eq!(records[1].level, log::Level::Error);
    // The rendered line carries timestamp, level, target, and message.
    let line = records[0].line();
    assert!(line.contains("INFO"), "line should show the level: {line}");
    assert!(line.contains("rixlcode::backend: turn started"), "line should show target + message: {line}");
}

#[test]
fn buffer_evicts_oldest_at_capacity() {
    reset();
    for ix in 0..logs::CAPACITY + 10 {
        logs::push(logs::record(log::Level::Info, "test", &format!("line {ix}")));
    }
    let records = logs::records();
    assert_eq!(records.len(), logs::CAPACITY, "buffer should cap at CAPACITY");
    assert_eq!(records[0].message, "line 10", "the oldest records should be evicted first");
    assert_eq!(records[logs::CAPACITY - 1].message, format!("line {}", logs::CAPACITY + 9));
}

#[test]
fn level_filter_selects_by_severity() {
    reset();
    logs::push(logs::record(log::Level::Debug, "test", "debug line"));
    logs::push(logs::record(log::Level::Info, "test", "info line"));
    logs::push(logs::record(log::Level::Warn, "test", "warn line"));
    logs::push(logs::record(log::Level::Error, "test", "error line"));

    assert_eq!(logs::filtered_lines(LogFilter::All).len(), 4);
    let info = logs::filtered_lines(LogFilter::Info);
    assert_eq!(info.len(), 3, "Info+ keeps info/warn/error, drops debug");
    assert!(!info.iter().any(|l| l.contains("debug line")));
    let warn = logs::filtered_lines(LogFilter::Warn);
    assert_eq!(warn.len(), 2);
    assert!(warn.iter().all(|l| l.contains("warn line") || l.contains("error line")));
    let error = logs::filtered_lines(LogFilter::Error);
    assert_eq!(error.len(), 1);
    assert!(error[0].contains("error line"));
}

/// Cmd-Shift-L opens the panel, the rows render the buffered lines, the
/// filter chips narrow them, Copy puts the visible lines on the clipboard,
/// and Esc closes.
#[test]
fn view_logs_panel() {
    reset();
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        cx.bind_keys(crate::panel_keys());
        window.draw(cx).clear(cx);
        assert!(window.try_find("logs-overlay").is_none(), "panel starts closed");

        logs::push(logs::record(log::Level::Info, "rixlcode::test", "hello from the log"));
        logs::push(logs::record(log::Level::Error, "rixlcode::test", "something broke"));

        window.press("cmd-shift-l", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).logs_open, "cmd-shift-l should open the logs panel");
        assert!(window.find("logs-overlay").visible());
        assert_eq!(
            window.find(("logs-row", 0usize)).label().unwrap_or_default(),
            logs::records()[0].line(),
            "row 0 should render the first record's line"
        );
        assert!(window.find(("logs-row", 1usize)).label().unwrap_or_default().contains("something broke"));

        // The Error chip narrows the list to the error record only.
        window.click("logs-filter-Error", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).logs_filter, LogFilter::Error);
        assert!(window.try_find(("logs-row", 0usize)).is_none(), "filtered-out rows should not render");
        assert!(window.find(("logs-row", 1usize)).label().unwrap_or_default().contains("something broke"));

        // Copy writes exactly the visible lines.
        window.click("logs-copy", cx);
        let copied = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert_eq!(copied, logs::filtered_lines(LogFilter::Error).join("\n"));
        assert!(copied.contains("something broke") && !copied.contains("hello from the log"));

        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).logs_open, "esc should close the logs panel");
        assert!(window.try_find("logs-overlay").is_none());
    });
}

/// The header ✕ and the dimmed backdrop both dismiss the panel.
#[test]
fn view_logs_closes_on_close_button_and_backdrop() {
    reset();
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        logs::push(logs::record(log::Level::Info, "test", "line"));
        ws.update(cx, |this, cx| this.toggle_logs(window, cx));
        window.draw(cx).clear(cx);
        window.click("logs-close", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).logs_open, "header close should dismiss the panel");

        ws.update(cx, |this, cx| this.toggle_logs(window, cx));
        window.draw(cx).clear(cx);
        // Center is occluded by the panel — click a corner of the backdrop.
        window.click_at("logs-backdrop", point(px(8.), px(8.)), cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).logs_open, "backdrop click should dismiss the panel");
    });
}

/// The palette exposes a View Logs command that dispatches the action.
#[test]
fn palette_lists_view_logs() {
    let entries = crate::palette_items::build_entries(&[], "view logs", 0);
    assert!(
        entries
            .iter()
            .any(|e| matches!(e, crate::palette_items::Entry::Command(spec) if spec.label == "View Logs")),
        "the palette should list a View Logs command"
    );
}
