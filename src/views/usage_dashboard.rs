//! The usage dashboard: a centered modal over a dimmed backdrop aggregating
//! token and cost totals across every chat in the window — headline totals,
//! a per-model table with share bars, and the ten priciest chats. Read-only:
//! the numbers come from `UsageTotals::gather` over the chats' folded
//! `ChatUsage`. Mounted by `Workspace::render` while
//! `Workspace::usage_dashboard_open` is set; Esc (via `escape_key` in
//! `root`), the header ✕, or a backdrop click closes it.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::pricing::fmt_cost;
use crate::usage::{ChatTotal, ChatUsageEntry, ModelTotal, UsageTotals, fmt_tokens};
use crate::workspace::Workspace;

/// The per-chat ranking caps at this many rows.
const TOP_CHATS: usize = 10;

impl Workspace {
    /// The palette's "Usage Dashboard" row and the popover's "View all":
    /// toggle the dashboard overlay.
    pub fn toggle_usage_dashboard(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.usage_dashboard_open = !self.usage_dashboard_open;
        cx.notify();
    }

    /// The dashboard's data: every chat folded into `UsageTotals`. A chat's
    /// cost is priced on its own model, falling back to the current
    /// selection for legacy chats (empty `model`) — same rule as
    /// `session_usage`.
    pub fn usage_totals(&self) -> UsageTotals {
        let entries: Vec<ChatUsageEntry<'_>> = self
            .chats
            .iter()
            .map(|chat| ChatUsageEntry {
                title: chat.title.as_ref(),
                model: if chat.model.is_empty() { self.model.as_ref() } else { chat.model.as_str() },
                usage: &chat.usage,
            })
            .collect();
        UsageTotals::gather(&entries)
    }
}

/// The overlay root: full-window backdrop + centered panel. The backdrop's
/// hitbox covers the window, so a press anywhere the panel doesn't occlude
/// lands on it and closes the panel.
pub(crate) fn usage_dashboard_overlay(this: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let backdrop = div()
        .id("usage-dashboard-backdrop")
        .test_support()
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.usage_dashboard_open = false;
                cx.notify();
            }),
        );
    div()
        .id("usage-dashboard-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .child(backdrop)
        .child(div().absolute().inset_0().flex().items_center().justify_center().child(panel(this, cx)))
}

/// The centered card: header above a scrollable column with the headline
/// stats, the per-model table, and the per-chat ranking.
fn panel(this: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let totals = this.usage_totals();
    let theme = cx.theme();
    div()
        .occlude()
        .w(px(640.))
        .h(px(480.))
        .flex()
        .flex_col()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded_lg()
        .shadow_lg()
        .child(header(cx))
        .child(
            v_flex()
                .id("usage-dashboard-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .gap_4()
                .child(summary(&totals, cx))
                .child(model_table(&totals, cx))
                .child(chat_table(&totals, cx)),
        )
}

fn header(cx: &mut Context<Workspace>) -> Div {
    let theme = cx.theme();
    h_flex()
        .gap_2()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(IconName::ChartPie)
                .child("Usage"),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("usage-dashboard-close")
                .test_support()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(IconName::X)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.usage_dashboard_open = false;
                    cx.notify();
                })),
        )
}

/// The headline row: tokens in, tokens out, estimated cost. A `+` suffix
/// marks the cost a lower bound when some chat's model is unpriced; `—`
/// when nothing priced anything.
fn summary(totals: &UsageTotals, cx: &App) -> impl IntoElement {
    let cost = if totals.cost_partial && totals.total_cost == 0. {
        "—".to_string()
    } else if totals.cost_partial {
        format!("~{}+", fmt_cost(totals.total_cost))
    } else {
        format!("~{}", fmt_cost(totals.total_cost))
    };
    h_flex()
        .id("usage-dashboard-summary")
        .test_support()
        .gap_6()
        .child(stat("usage-total-in", "Tokens in", fmt_tokens(totals.total_input), cx))
        .child(stat("usage-total-out", "Tokens out", fmt_tokens(totals.total_output), cx))
        .child(stat("usage-total-cost", "Est. cost", cost, cx))
}

/// One headline figure — `aria_label` carries `label: value` so headless
/// tests can assert it.
fn stat(id: impl Into<ElementId>, label: &'static str, value: String, cx: &App) -> impl IntoElement {
    v_flex()
        .id(id)
        .test_support()
        .aria_label(format!("{label}: {value}"))
        .gap_0p5()
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(value))
}

/// The per-model table: model, token split, estimated cost, and a share
/// bar. The bar reads as cost share while any chat priced; when nothing
/// did it falls back to token share so the column still renders.
fn model_table(totals: &UsageTotals, cx: &App) -> impl IntoElement {
    let rows: Vec<AnyElement> = totals.by_model.iter().enumerate().map(|(ix, m)| model_row(ix, m, totals, cx)).collect();
    v_flex()
        .id("usage-dashboard-models")
        .test_support()
        .gap_1()
        .child(section_title("By model", cx))
        .when(rows.is_empty(), |d| d.child(empty_note("No usage yet — send a message first", cx)))
        .children(rows)
}

fn model_row(ix: usize, m: &ModelTotal, totals: &UsageTotals, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let tokens = token_split(m);
    let cost = m.cost.map(|c| format!("~{}", fmt_cost(c))).unwrap_or_else(|| "—".into());
    h_flex()
        .id(("usage-model", ix))
        .test_support()
        .aria_label(format!("{}: {tokens} · {cost}", m.model))
        .gap_3()
        .py_1()
        .text_xs()
        .child(div().w(px(160.)).flex_shrink_0().text_ellipsis().child(m.model.clone()))
        .child(div().w(px(150.)).flex_shrink_0().text_color(theme.muted_foreground).child(tokens))
        .child(div().w(px(80.)).flex_shrink_0().child(cost))
        .child(share_bar(m, totals, cx))
        .into_any_element()
}

/// `300 in · 150 out`, plus `· 60 cached` when the model's chats reported
/// cache tokens.
fn token_split(m: &ModelTotal) -> String {
    let mut s = format!("{} in · {} out", fmt_tokens(m.tokens.input), fmt_tokens(m.tokens.output));
    if m.tokens.cached > 0 {
        s.push_str(&format!(" · {} cached", fmt_tokens(m.tokens.cached)));
    }
    s
}

/// The row's share of the whole: cost share while `total_cost` is nonzero,
/// token share otherwise (an unpriced model can't claim a cost share).
fn share_bar(m: &ModelTotal, totals: &UsageTotals, cx: &App) -> Div {
    let frac = if totals.total_cost > 0. {
        (m.cost.unwrap_or(0.) / totals.total_cost) as f32
    } else {
        let total = totals.total_input + totals.total_output;
        if total == 0 { 0. } else { m.tokens.total() as f32 / total as f32 }
    };
    div()
        .flex_1()
        .h(px(6.))
        .rounded_full()
        .bg(cx.theme().accent.alpha(0.12))
        .child(div().h_full().w(relative(frac.clamp(0., 1.))).rounded_full().bg(cx.theme().accent))
}

/// The per-chat ranking: the ten priciest chats, title plus tokens and
/// estimate. Unpriced chats sort last and show `—`.
fn chat_table(totals: &UsageTotals, cx: &App) -> impl IntoElement {
    let rows: Vec<AnyElement> = totals.by_chat.iter().take(TOP_CHATS).enumerate().map(|(ix, c)| chat_row(ix, c, cx)).collect();
    v_flex()
        .id("usage-dashboard-chats")
        .test_support()
        .gap_1()
        .child(section_title("Top chats", cx))
        .when(rows.is_empty(), |d| d.child(empty_note("No chats have recorded usage", cx)))
        .children(rows)
}

fn chat_row(ix: usize, c: &ChatTotal, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let cost = c.cost.map(|v| format!("~{}", fmt_cost(v))).unwrap_or_else(|| "—".into());
    h_flex()
        .id(("usage-chat", ix))
        .test_support()
        .aria_label(format!("{}: {} tok · {cost}", c.title, c.tokens))
        .gap_3()
        .py_1()
        .text_xs()
        .child(div().flex_1().min_w_0().text_ellipsis().child(c.title.clone()))
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(format!("{} tok", fmt_tokens(c.tokens))),
        )
        .child(div().w(px(80.)).flex_shrink_0().child(cost))
        .into_any_element()
}

fn section_title(label: &'static str, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

fn empty_note(text: &'static str, cx: &App) -> Div {
    div().py_2().text_sm().text_color(cx.theme().muted_foreground).child(text)
}

#[cfg(test)]
#[path = "usage_dashboard_tests.rs"]
mod usage_dashboard_tests;
