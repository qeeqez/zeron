//! The View Logs panel: a centered modal over a dimmed backdrop listing the
//! app's captured log records (see `crate::logs`) in monospace, with a
//! minimum-level filter, a Copy button for the visible lines, and a Reveal
//! button for the on-disk log file. Mounted by `Workspace::render` while
//! `Workspace::logs_open` is set; Esc (via `escape_key` in `root`), the
//! header ✕, or a backdrop click closes it.

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::logs::{self, LogFilter};
use crate::workspace::Workspace;

impl Workspace {
    /// Cmd-Shift-L / View > View Logs / the palette row: toggle the logs
    /// overlay — a centered panel rendered while `logs_open` is set.
    pub fn toggle_logs(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.logs_open = !self.logs_open;
        cx.notify();
    }

    /// The header's level chips — re-renders the rows under the new filter.
    pub fn set_logs_filter(&mut self, filter: LogFilter, cx: &mut Context<Self>) {
        self.logs_filter = filter;
        cx.notify();
    }

    /// Copy the visible (filtered) lines to the clipboard — one line per
    /// record, same rendering the panel shows.
    pub fn copy_logs(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(logs::filtered_lines(self.logs_filter).join("\n")));
    }

    /// Reveal the on-disk log file in the platform's file manager.
    pub fn reveal_log_file(&mut self, cx: &mut Context<Self>) {
        cx.reveal_path(&logs::log_file_path());
    }
}

/// The overlay root: full-window backdrop + centered panel. The backdrop's
/// hitbox covers the window, so a press anywhere the panel doesn't occlude
/// lands on it and closes the panel.
pub(crate) fn logs_overlay(this: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let backdrop = div()
        .id("logs-backdrop")
        .test_support()
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.logs_open = false;
                cx.notify();
            }),
        );
    div()
        .id("logs-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .child(backdrop)
        .child(div().absolute().inset_0().flex().items_center().justify_center().child(panel(this, cx)))
}

/// The centered card: header (title, filter chips, actions, close) above a
/// scrollable column of log lines.
fn panel(this: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let theme = cx.theme();
    div()
        .occlude()
        .w(px(760.))
        .h(px(480.))
        .flex()
        .flex_col()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded_lg()
        .shadow_lg()
        .child(header(this, cx))
        .child(rows(this, cx))
}

fn header(this: &Workspace, cx: &mut Context<Workspace>) -> Div {
    // Chips first: building them reborrows `cx`, so `theme` must come after.
    let chips = LogFilter::ALL.map(|filter| filter_chip(this, filter, cx));
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(IconName::ScrollText)
                .child("Logs"),
        )
        .children(chips)
        .child(
            Button::new("logs-copy")
                .label("Copy")
                .icon(IconName::Copy)
                .small()
                .outline()
                .tooltip("Copy the visible lines")
                .on_click(cx.listener(|this, _, _, cx| this.copy_logs(cx))),
        )
        .child(
            Button::new("logs-reveal")
                .icon(IconName::FolderOpen)
                .small()
                .ghost()
                .tooltip("Reveal the log file")
                .on_click(cx.listener(|this, _, _, cx| this.reveal_log_file(cx))),
        )
        .child(
            div()
                .id("logs-close")
                .test_support()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(IconName::X)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.logs_open = false;
                    cx.notify();
                })),
        )
}

/// One filter chip — the active level reads as a filled pill, the rest as
/// plain text buttons.
fn filter_chip(this: &Workspace, filter: LogFilter, cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    let theme = cx.theme();
    let active = this.logs_filter == filter;
    div()
        .id(SharedString::from(format!("logs-filter-{}", filter.label())))
        .test_support()
        .cursor_pointer()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .when(active, |d| d.bg(theme.accent).text_color(theme.accent_foreground))
        .when(!active, |d| d.text_color(theme.muted_foreground))
        .child(filter.label())
        .on_click(cx.listener(move |this, _, _, cx| this.set_logs_filter(filter, cx)))
}

/// The scrollable log lines — monospace, colored by level, one row per
/// record passing the filter. `aria_label` carries the line so headless
/// tests can assert content.
fn rows(this: &Workspace, cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    let theme = cx.theme();
    let records = logs::records();
    let lines: Vec<(usize, &logs::LogRecord)> = records.iter().enumerate().filter(|(_, r)| this.logs_filter.allows(r.level)).collect();
    if lines.is_empty() {
        return div()
            .id("logs-empty")
            .test_support()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("No log records");
    }
    let mut list = div()
        .id("logs-rows")
        .test_support()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .p_2()
        .overflow_y_scroll()
        .font_family(theme.mono_font_family.clone())
        .text_xs();
    for (ix, record) in lines {
        let line = record.line();
        list = list.child(
            div()
                .id(("logs-row", ix))
                .test_support()
                .aria_label(line.clone())
                .text_color(level_color(record.level, cx))
                .child(line),
        );
    }
    list
}

fn level_color(level: log::Level, cx: &App) -> Hsla {
    let theme = cx.theme();
    match level {
        log::Level::Error => theme.danger,
        log::Level::Warn => theme.warning,
        log::Level::Info => theme.info,
        log::Level::Debug | log::Level::Trace => theme.muted_foreground,
    }
}
