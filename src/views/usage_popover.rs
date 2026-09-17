//! The usage breakdown popover behind the composer meter: per-chat totals,
//! context fill, estimated cost (only when the model's pricing is known),
//! a per-turn token list, and the session total across chats. Built on the
//! unstyled `base::Popover` so the meter itself stays the trigger — the
//! styled `component::Popover` only accepts `Selectable` triggers.

use gpui_kit::assets::IconName;
use gpui_kit::base::Popover;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{ThemeStyled, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::pricing::fmt_cost;
use crate::usage::{ChatUsage, SessionUsage, TurnUsage, fmt_tokens};
use crate::workspace::Workspace;

/// The composer meter wrapped as a popover trigger. `None` until the first
/// usage report or rate-limit snapshot — an untouched thread shows nothing.
/// With no token meter yet, a quota snapshot still gets a trigger: a
/// warning glyph that opens the same breakdown.
pub fn usage_popover(usage: &ChatUsage, ws: &Entity<Workspace>, cx: &App) -> Option<impl IntoElement> {
    // into_any_element erases the opaque type's captured lifetimes — the
    // trigger closure must be 'static.
    let trigger = match super::composer_helpers::usage_indicator(usage, cx) {
        Some(meter) => meter.into_any_element(),
        None if usage.rate_limit.is_some() => div()
            .id("usage-meter")
            .test_support()
            .text_xs()
            .text_color(cx.theme().warning)
            .child(IconName::TriangleAlert)
            .into_any_element(),
        None => return None,
    };
    let ws = ws.clone();
    Some(
        Popover::new("usage-popover")
            .anchor(Anchor::BottomRight)
            .trigger_with(move |_, _, _| div().cursor_pointer().child(trigger).into_any_element())
            .content(move |_, _, cx| usage_breakdown(&ws, cx)),
    )
}

/// The popover's styled surface: stat rows, then one row per turn, then
/// the session total when the window holds more than one chat.
fn usage_breakdown(ws: &Entity<Workspace>, cx: &mut Context<gpui_kit::base::PopoverState>) -> AnyElement {
    let popover = cx.entity();
    let ws_entity = ws.clone();
    let ws = ws.read(cx);
    let chat = &ws.chats[ws.active];
    let usage = &chat.usage;
    let model = if chat.model.is_empty() { ws.model.as_ref() } else { chat.model.as_str() };
    let cost = usage.cost(model);
    let session = (ws.chats.len() > 1).then(|| ws.session_usage());
    let muted = cx.theme().muted_foreground;

    let mut body = v_flex()
        .id("usage-breakdown")
        .test_support()
        .gap_1()
        .w(px(240.))
        .text_xs()
        .child(div().text_color(muted).child("Usage"))
        .child(stat_row("usage-total", "This chat", format!("{} tok", fmt_tokens(usage.total)), cx));
    if let Some(fill) = usage.fill() {
        let used = usage.context_used.unwrap_or(usage.total);
        body = body.child(stat_row(
            "usage-context",
            "Context",
            format!("{} / {} ({:.0}%)", fmt_tokens(used), fmt_tokens(usage.context.unwrap_or(0)), fill * 100.),
            cx,
        ));
    }
    // Quota windows from the backend's rate-limit snapshot, plus a
    // "reached" row while a limit is active.
    if let Some(rl) = &usage.rate_limit {
        if rl.limited {
            body = body.child(stat_row("usage-limit", "Rate limit", "reached".into(), cx));
        }
        for (ix, w) in rl.windows().enumerate() {
            body = body.child(stat_row(("usage-quota", ix), w.label(), w.value(std::time::SystemTime::now()), cx));
        }
    }
    // Honest cost: a priced model shows the estimate, anything else '—'.
    let cost_text = cost.map(|c| format!("~{}", fmt_cost(c))).unwrap_or_else(|| "—".into());
    body = body.child(stat_row("usage-cost", "Est. cost", cost_text, cx));
    for (ix, t) in usage.turn_rows().iter().enumerate() {
        body = body.child(stat_row(("usage-turn", ix), format!("Turn {}", ix + 1), token_split(*t), cx));
    }
    if let Some(s) = session {
        body = body.child(session_row(s, cx));
    }
    body = body.child(view_all_row(&ws_entity, &popover, cx));
    body.popover_style(cx).p_3().bottom_1().into_any_element()
}

/// The footer row: dismisses the popover and opens the usage panel.
fn view_all_row(ws: &Entity<Workspace>, popover: &Entity<gpui_kit::base::PopoverState>, cx: &App) -> impl IntoElement {
    let ws = ws.clone();
    let popover = popover.clone();
    h_flex()
        .id("usage-view-all")
        .test_support()
        .mt_1()
        .pt_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .hover(|d| d.text_color(cx.theme().foreground))
        .child(IconName::ChartPie)
        .child("View all usage")
        .child(div().flex_1())
        .child(IconName::ArrowRight)
        .on_click(move |_, window, cx| {
            popover.update(cx, |state, cx| state.dismiss(window, cx));
            ws.update(cx, |this, cx| this.toggle_usage_panel(window, cx));
        })
}

/// One label/value row — `id` is the test/click target.
fn stat_row(id: impl Into<ElementId>, label: impl Into<SharedString>, value: String, cx: &App) -> impl IntoElement {
    let label = label.into();
    h_flex()
        .id(id)
        .test_support()
        .aria_label(format!("{label}: {value}"))
        .justify_between()
        .gap_2()
        .child(div().text_color(cx.theme().muted_foreground).child(label))
        .child(div().child(value))
}

/// A turn's token split: `300 in · 150 out`, plus `· 60 cached` when the
/// backend reports cache tokens.
pub(crate) fn token_split(t: TurnUsage) -> String {
    let mut s = format!("{} in · {} out", fmt_tokens(t.input), fmt_tokens(t.output));
    if t.cached > 0 {
        s.push_str(&format!(" · {} cached", fmt_tokens(t.cached)));
    }
    s
}

/// The session row: total tokens across chats plus the summed estimate —
/// a `+` suffix marks it a lower bound when some chat's model is unpriced.
fn session_row(s: SessionUsage, cx: &App) -> impl IntoElement {
    let cost = if s.cost_partial && s.cost == 0. {
        "—".to_string()
    } else if s.cost_partial {
        format!("~{}+", fmt_cost(s.cost))
    } else {
        format!("~{}", fmt_cost(s.cost))
    };
    stat_row("usage-session", "Session", format!("{} tok · {}", fmt_tokens(s.total), cost), cx)
}
