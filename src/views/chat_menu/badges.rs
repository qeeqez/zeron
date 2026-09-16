//! Titlebar chips + the color dot — split from `chat_menu.rs` for the SLOC
//! cap (same pattern as `chat_menu/color.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

/// The color-tag dot — one shape for the sidebar row, the titlebar and the
/// ⋯ menu's swatches. `id` keeps it findable in headless tests.
pub fn color_dot(id: impl Into<ElementId>, color: crate::model::ChatColor, size: Pixels) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .w(size)
        .h(size)
        .rounded_full()
        .flex_shrink_0()
        .bg(color.hsla())
        .into_any_element()
}

/// The worktree chip on the chat titlebar — a muted icon + "worktree" label
/// whose tooltip carries the checkout path. `id` keeps it findable in
/// headless tests.
pub fn worktree_badge(id: &'static str, workdir: &str, cx: &App) -> AnyElement {
    let tip = workdir.to_string();
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::FolderGit)
        .child("worktree")
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .into_any_element()
}

/// The "Temporary" chip on the chat titlebar — same muted styling as the
/// worktree badge; a chat can carry both. `id` keeps it findable in
/// headless tests.
pub fn temp_badge(id: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::Ghost)
        .child("Temporary")
        .tooltip(|window, cx| Tooltip::new("Not saved — closes with the chat").build(window, cx))
        .into_any_element()
}

/// The context-window meter on the chat titlebar — `NN%` of the window
/// used (Codex-style; the tooltip carries the exact counts), muted under
/// 80%, warning at 80%+, danger at 95%+. Token backends report no window
/// size, so the chip falls back to the chat's cumulative token count and
/// stays muted. `None` until the first usage report — a fresh chat or a
/// backend without usage (sim) shows nothing.
pub fn context_chip(id: &'static str, usage: &crate::usage::ChatUsage, cx: &App) -> Option<AnyElement> {
    use crate::usage::{ContextMeter, MeterTier};
    let meter = usage.meter()?;
    let color = match meter {
        ContextMeter::Fill { tier: MeterTier::Warning, .. } => cx.theme().warning,
        ContextMeter::Fill { tier: MeterTier::Danger, .. } => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
    let tip = meter.detail();
    Some(
        div()
            .id(id)
            .test_support()
            .aria_label(tip.clone())
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded_md()
            .text_xs()
            .text_color(color)
            .child(IconName::CircleGauge)
            .child(meter.label())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element(),
    )
}
