//! Day separators in the transcript — a centered muted label ("Today",
//! "Yesterday", "Mon, Mar 3") above the first visible message of each new
//! day, ChatGPT/Codex style. The separator renders inside that message's
//! scroller row rather than as a synthetic row of its own: every scroller
//! callsite (`filtered_pos`, `visible_to_real`, `append`, `remeasure_items`,
//! `scroll_to_item`, the test helpers' `reset(len)`) is built on the
//! invariant that row `i` shows the `i`-th visible message, and keeping it
//! leaves all of them — nav cursor, find hits, jump-to-match — untouched.
//!
//! Boundaries use local dates (`ChatMessage.at` → `chrono::Local`, the same
//! conversion as `timestamp_label` below). The leading message of the
//! transcript gets no label — separators only divide days.
//!
//! Declared from `views/mod.rs` via `#[path]` — `main.rs` is at the SLOC cap.
use std::time::SystemTime;

use chrono::Datelike;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// The divider text for a message at `at` following `prev_at` — `None` when
/// both fall on the same local day, or when there is no previous visible
/// message (the transcript's first row never gets a header).
pub(crate) fn separator_label(prev_at: Option<SystemTime>, at: SystemTime) -> Option<String> {
    let prev = chrono::DateTime::<chrono::Local>::from(prev_at?).date_naive();
    let day = chrono::DateTime::<chrono::Local>::from(at).date_naive();
    (day != prev).then(|| day_label(day, chrono::Local::now().date_naive()))
}

/// "Today" / "Yesterday" / "Mon, Mar 3" — the year appended only when the
/// day isn't in `today`'s year. `pub(crate)` for the usage chart's labels.
pub(crate) fn day_label(day: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    if day == today {
        return "Today".into();
    }
    if today.pred_opt().is_some_and(|prev| prev == day) {
        return "Yesterday".into();
    }
    if day.year() == today.year() {
        day.format("%a, %b %-d").to_string()
    } else {
        day.format("%a, %b %-d, %Y").to_string()
    }
}

/// The message footer's absolute timestamp: "14:32" today, "Mon 14:32"
/// within the last week, "Sep 3" older (year appended off `today`'s year).
/// `today` is a parameter so tests pin the boundaries.
pub(crate) fn timestamp_label(at: SystemTime, today: chrono::NaiveDate) -> String {
    let dt = chrono::DateTime::<chrono::Local>::from(at);
    let day = dt.date_naive();
    if day == today {
        return dt.format("%H:%M").to_string();
    }
    if (today - day).num_days() < 7 {
        return dt.format("%a %H:%M").to_string();
    }
    if day.year() == today.year() {
        day.format("%b %-d").to_string()
    } else {
        day.format("%b %-d, %Y").to_string()
    }
}

/// Inputs for [`separator_row`] — bundled to stay under the arg-count lint.
/// `at` is the row's own timestamp, `prev_at` the previous visible
/// message's — under a chat-search filter that's the previous *match*, so a
/// divider still marks the day change between hits. `compact` shrinks the
/// divider's padding (see `message::density`).
pub(crate) struct SeparatorRow {
    pub ix: usize,
    pub at: Option<SystemTime>,
    pub prev_at: Option<SystemTime>,
    pub row: AnyElement,
    pub compact: bool,
}

/// Wrap a rendered message row in a column that carries the day separator
/// above it.
pub(crate) fn separator_row(s: SeparatorRow, cx: &App) -> AnyElement {
    let Some(label) = s.at.and_then(|at| separator_label(s.prev_at, at)) else {
        return s.row;
    };
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .id(("date-separator", s.ix))
                .test_support()
                .aria_label(label.clone())
                .flex()
                .justify_center()
                .py(crate::views::message::density(s.compact).separator_py)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(s.row)
        .into_any_element()
}

#[cfg(test)]
#[path = "date_separator_tests.rs"]
mod date_separator_tests;
