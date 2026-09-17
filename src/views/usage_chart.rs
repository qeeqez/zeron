//! The usage panel's daily bar chart: one column per day over the last
//! `CHART_DAYS` days, height proportional to that day's usage — estimated
//! cost while any day priced, tokens otherwise (mirroring the breakdown
//! rows' rule). Today's bar wears the accent color; empty days render
//! as gaps. Pure divs — no chart crate. Each bar's aria label and tooltip
//! carry the day's total.

use chrono::Datelike;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::pricing::fmt_cost;
use crate::usage::{DayTotal, fmt_tokens};

/// Weekday letters under the bars, Monday first — `num_days_from_monday`
/// indexes it, so no locale-formatted names.
const DAY_LETTERS: [char; 7] = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];

/// The chart section: title above a row of day columns. Bars scale to the
/// window's peak day; a zero day leaves a gap.
pub(crate) fn usage_chart(days: &[DayTotal], cx: &App) -> impl IntoElement {
    let cost_mode = days.iter().any(|d| d.cost.is_some_and(|c| c > 0.));
    let peak = days
        .iter()
        .map(|d| if cost_mode { d.cost.unwrap_or(0.) } else { d.tokens as f64 })
        .fold(0., f64::max);
    let bars: Vec<AnyElement> = days.iter().map(|d| bar(d, peak, cost_mode, cx)).collect();
    v_flex()
        .id("usage-chart")
        .test_support()
        .gap_1()
        .child(crate::views::usage_panel::section_title("Last 14 days", cx))
        .child(h_flex().h(px(72.)).items_stretch().gap_1().children(bars))
}

/// One day column: the proportional fill bottom-anchored in the track, a
/// weekday letter beneath. `frac` days under ~3% still get a 2px sliver so
/// nonzero usage never rounds to a gap.
fn bar(d: &DayTotal, peak: f64, cost_mode: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let today = d.day == chrono::Local::now().date_naive();
    let value = if cost_mode { d.cost.unwrap_or(0.) } else { d.tokens as f64 };
    let frac = if peak > 0. { (value / peak) as f32 } else { 0. };
    let mut label =
        format!("{}: {} tok", crate::views::date_separator::day_label(d.day, chrono::Local::now().date_naive()), fmt_tokens(d.tokens));
    if let Some(c) = d.cost {
        label.push_str(&format!(" · ~{}", fmt_cost(c)));
    }
    let fill = if today { theme.accent } else { theme.accent.alpha(0.35) };
    v_flex()
        .id(format!("usage-day-{}", d.day))
        .test_support()
        .aria_label(label.clone())
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .flex_1()
        .h_full()
        .child(
            div().flex_1().min_h_0().flex().flex_col().justify_end().child(
                div()
                    .id(format!("usage-day-fill-{}", d.day))
                    .test_support()
                    .w_full()
                    .h(relative(frac.clamp(0., 1.)))
                    .when(frac > 0., |d| d.min_h(px(2.)))
                    .rounded_t(px(2.))
                    .bg(fill),
            ),
        )
        .child(
            div()
                .h(px(12.))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .text_xs()
                .text_color(if today { theme.accent } else { theme.muted_foreground })
                .child(DAY_LETTERS[d.day.weekday().num_days_from_monday() as usize].to_string()),
        )
        .into_any_element()
}
