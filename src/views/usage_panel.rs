//! The Usage panel — a persistent right-side view of token and cost totals
//! across every chat in the window: today's persisted usage, this session's
//! headline totals, the two-week daily chart, per-provider and per-model
//! breakdowns, and the five priciest chats. Read-only: the numbers come
//! from `UsageTotals::gather` over the chats' folded `ChatUsage` and
//! persisted message stamps. Mounted by `Workspace::render` while
//! `Workspace::usage_panel_open` is set (persisted via
//! `Settings.usage_panel_open`); Esc (via `escape_key` in `root`), the
//! header ✕, or the sidebar row closes it.
//!
//! Scope note: `ChatUsage` carries no timestamps, so "This session" totals
//! cover the runtime counters — loaded chats contribute again only after a
//! new turn. "Today" and the chart read the persisted per-message `usage`
//! stamps instead, so they survive restarts but lag an in-flight turn
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::pricing::fmt_cost;
use crate::usage::{BreakdownRow, ChatTotal, ChatUsageEntry, UsageTotals, fmt_tokens};
use crate::workspace::Workspace;

/// The per-chat ranking caps at this many rows.
const TOP_CHATS: usize = 5;

impl Workspace {
    /// The sidebar's Usage row, the palette's "Usage Panel" command, and
    /// the popover's "View all" all land here. The flag persists like the
    /// other side panels' (`Settings.usage_panel_open`).
    pub fn toggle_usage_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.usage_panel_open = !self.usage_panel_open;
        // The panel totals per-message token stamps — opening it is the
        // user's request for that data, so unopened chats hydrate here.
        if self.usage_panel_open {
            self.ensure_all_messages();
        }
        self.save_settings();
        cx.notify();
    }

    /// The panel's data: every chat folded into `UsageTotals`. A chat's
    /// cost is priced on its own model, falling back to the current
    /// selection for legacy chats (empty `model`) — same rule as
    /// `session_usage`. The provider row labels by the instance's name
    /// (a deleted instance falls back to its id). The daily chart reads
    /// the persisted per-message usage stamps; acp chats pass no
    /// messages — their stamps record context occupancy, not tokens
    /// (files written before the stamp fix).
    pub fn usage_totals(&self) -> UsageTotals {
        let entries: Vec<ChatUsageEntry<'_>> = self
            .chats
            .iter()
            .map(|chat| {
                let provider = if chat.provider.is_empty() { self.selected_provider.as_str() } else { chat.provider.as_str() };
                let instance = self.providers.iter().find(|p| p.id == provider);
                let counts_tokens = instance.is_none_or(|p| p.kind != crate::providers::ProviderKind::Acp);
                ChatUsageEntry {
                    title: chat.title.as_ref(),
                    model: if chat.model.is_empty() { self.model.as_ref() } else { chat.model.as_str() },
                    provider: instance.map_or(provider, |p| p.name.as_str()),
                    usage: &chat.usage,
                    messages: if counts_tokens { chat.messages.as_slice() } else { &[] },
                }
            })
            .collect();
        UsageTotals::gather(&entries, chrono::Local::now().date_naive())
    }

    pub fn render_usage_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let totals = self.usage_totals();
        // Nothing recorded at all — no session tokens and no stamped
        // history — collapses the sections into one empty note.
        let empty = totals.by_chat.is_empty() && totals.by_day.iter().all(|d| d.tokens == 0);
        let theme = cx.theme();
        div()
            .id("usage-panel")
            .test_support()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .font_bold()
                    .child(IconName::ChartPie)
                    .child("Usage")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("usage-panel-close")
                            .test_support()
                            .cursor_pointer()
                            .text_color(theme.muted_foreground)
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_usage_panel(window, cx))),
                    ),
            )
            .child(
                v_flex()
                    .id("usage-panel-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .gap_4()
                    .when(empty, |d| {
                        d.child(
                            div()
                                .id("usage-panel-empty")
                                .test_support()
                                .aria_label("No usage yet")
                                .py_2()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child("No usage yet — send a message first."),
                        )
                    })
                    .when(!empty, |d| {
                        d.child(today_section(&totals, cx))
                            .child(session_section(&totals, cx))
                            .child(crate::views::usage_chart::usage_chart(&totals.by_day, cx))
                            .child(breakdown_table("usage-panel-providers", "usage-provider", "By provider", &totals.by_provider, cx))
                            .child(breakdown_table("usage-panel-models", "usage-model", "By model", &totals.by_model, cx))
                            .child(chat_table(&totals, cx))
                    }),
            )
    }
}

/// Today's persisted usage — the last `by_day` slot. `None` cost means no
/// stamped turn priced today.
fn today_section(totals: &UsageTotals, cx: &App) -> impl IntoElement {
    let today = totals.by_day.last();
    let tokens = today.map_or(0, |d| d.tokens);
    let cost = today.and_then(|d| d.cost);
    let value = match cost {
        Some(c) => format!("{} tok · ~{}", fmt_tokens(tokens), fmt_cost(c)),
        None => format!("{} tok", fmt_tokens(tokens)),
    };
    v_flex()
        .id("usage-panel-today")
        .test_support()
        .aria_label(format!("Today: {value}"))
        .gap_1()
        .child(section_title("Today", cx))
        .child(div().text_sm().child(value))
}

/// The session headline: tokens in, tokens out, estimated cost across the
/// runtime counters. A `+` suffix marks the cost a lower bound when some
/// chat's model is unpriced; `—` when nothing priced anything.
fn session_section(totals: &UsageTotals, cx: &App) -> impl IntoElement {
    let cost = if totals.cost_partial && totals.total_cost == 0. {
        "—".to_string()
    } else if totals.cost_partial {
        format!("~{}+", fmt_cost(totals.total_cost))
    } else {
        format!("~{}", fmt_cost(totals.total_cost))
    };
    v_flex()
        .id("usage-panel-summary")
        .test_support()
        .gap_1()
        .child(section_title("This session", cx))
        .child(stat("usage-total-in", "Tokens in", fmt_tokens(totals.total_input), cx))
        .child(stat("usage-total-out", "Tokens out", fmt_tokens(totals.total_output), cx))
        .child(stat("usage-total-cost", "Est. cost", cost, cx))
}

/// One headline figure — `aria_label` carries `label: value` so headless
/// tests can assert it.
fn stat(id: impl Into<ElementId>, label: &'static str, value: String, cx: &App) -> impl IntoElement {
    h_flex()
        .id(id)
        .test_support()
        .aria_label(format!("{label}: {value}"))
        .justify_between()
        .gap_2()
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(value))
}

/// A breakdown table — per-provider and per-model share the row shape:
/// label over a muted token split, the estimate right-aligned. `—` marks
/// a row whose pricing is unknown.
fn breakdown_table(id: &'static str, row_id: &'static str, title: &'static str, rows: &[BreakdownRow], cx: &App) -> impl IntoElement {
    let rows: Vec<AnyElement> = rows.iter().enumerate().map(|(ix, r)| breakdown_row(row_id, ix, r, cx)).collect();
    v_flex().id(id).test_support().gap_1().child(section_title(title, cx)).children(rows)
}

fn breakdown_row(row_id: &'static str, ix: usize, r: &BreakdownRow, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let tokens = crate::views::usage_popover::token_split(r.tokens);
    let cost = r.cost.map(|c| format!("~{}", fmt_cost(c))).unwrap_or_else(|| "—".into());
    v_flex()
        .id((row_id, ix))
        .test_support()
        .aria_label(format!("{}: {tokens} · {cost}", r.label))
        .py_1()
        .gap_0p5()
        .child(
            h_flex()
                .gap_2()
                .text_xs()
                .child(div().flex_1().min_w_0().text_ellipsis().child(r.label.clone()))
                .child(div().flex_shrink_0().child(cost)),
        )
        .child(div().text_xs().text_color(theme.muted_foreground).child(tokens))
        .into_any_element()
}

/// The per-chat ranking: the five priciest chats, title plus tokens and
/// estimate. Unpriced chats sort last and show `—`.
fn chat_table(totals: &UsageTotals, cx: &App) -> impl IntoElement {
    let rows: Vec<AnyElement> = totals.by_chat.iter().take(TOP_CHATS).enumerate().map(|(ix, c)| chat_row(ix, c, cx)).collect();
    v_flex()
        .id("usage-panel-chats")
        .test_support()
        .gap_1()
        .child(section_title("Top chats", cx))
        .children(rows)
}

fn chat_row(ix: usize, c: &ChatTotal, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let cost = c.cost.map(|v| format!("~{}", fmt_cost(v))).unwrap_or_else(|| "—".into());
    h_flex()
        .id(("usage-chat", ix))
        .test_support()
        .aria_label(format!("{}: {} tok · {cost}", c.title, c.tokens))
        .gap_2()
        .py_1()
        .text_xs()
        .child(div().flex_1().min_w_0().text_ellipsis().child(c.title.clone()))
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(format!("{} tok", fmt_tokens(c.tokens))),
        )
        .child(div().w(px(56.)).flex_shrink_0().text_right().child(cost))
        .into_any_element()
}

pub(crate) fn section_title(label: &'static str, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

/// The sidebar's Usage row — opens the panel; the suffix carries the
/// session's estimated cost (tokens when nothing priced) so spend is
/// glanceable while closed. Extracted so `sidebar.rs` stays under the
/// SLOC cap.
pub(crate) fn usage_nav_row(ws: &Workspace, cx: &mut Context<Workspace>) -> super::nav_row::NavRow {
    let session = ws.session_usage();
    let suffix = if session.cost > 0. {
        format!("~{}", crate::pricing::fmt_cost_compact(session.cost))
    } else if session.total > 0 {
        format!("{} tok", fmt_tokens(session.total))
    } else {
        String::new()
    };
    super::nav_row::NavRow::new("sidebar-usage", "Usage")
        .icon(IconName::ChartPie)
        .active(ws.usage_panel_open)
        .suffix(move |_, cx| {
            div()
                .id("sidebar-usage-total")
                .test_support()
                .text_xs()
                .when(!suffix.is_empty(), |d| d.aria_label(suffix.clone()).text_color(cx.theme().muted_foreground).child(suffix.clone()))
        })
        .on_click(cx.listener(|this, _, window, cx| this.toggle_usage_panel(window, cx)))
}

#[cfg(test)]
#[path = "usage_panel_tests.rs"]
mod usage_panel_tests;
