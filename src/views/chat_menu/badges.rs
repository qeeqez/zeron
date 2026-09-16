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
/// stays muted. A priced `model` appends the running cost estimate —
/// `62% · $0.43` — and the tooltip adds the rate; unpriced models (local
/// or free providers, unknown ids) show tokens only. `None` until the
/// first usage report — a fresh chat or a backend without usage (sim)
/// shows nothing.
pub fn context_chip(id: &'static str, usage: &crate::usage::ChatUsage, model: &str, cx: &App) -> Option<AnyElement> {
    use crate::usage::{ContextMeter, MeterTier};
    let meter = usage.meter()?;
    let color = match meter {
        ContextMeter::Fill { tier: MeterTier::Warning, .. } => cx.theme().warning,
        ContextMeter::Fill { tier: MeterTier::Danger, .. } => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
    let tip = chip_tooltip(&meter, usage, model);
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
            .child(chip_label(&meter, usage, model))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element(),
    )
}

/// The chip's visible text: the meter label plus ` · $0.43` when the
/// model's rates are known — `None` (no suffix) for unpriced models.
fn chip_label(meter: &crate::usage::ContextMeter, usage: &crate::usage::ChatUsage, model: &str) -> String {
    match usage.cost(model) {
        Some(cost) => format!("{} · {}", meter.label(), crate::pricing::fmt_cost_compact(cost)),
        None => meter.label(),
    }
}

/// Tooltip + aria label: the meter's exact counts, then the estimate and
/// the rate behind it when the model is priced.
fn chip_tooltip(meter: &crate::usage::ContextMeter, usage: &crate::usage::ChatUsage, model: &str) -> String {
    let detail = meter.detail();
    let Some(pricing) = crate::pricing::model_pricing(model) else { return detail };
    let cost = crate::pricing::fmt_cost(pricing.cost(usage.tokens()));
    format!("{detail} · ~{cost} est · ${} in / ${} out per Mtok", pricing.input, pricing.output)
}

#[cfg(test)]
mod tests {
    use super::{chip_label, chip_tooltip};
    use crate::usage::{ChatUsage, UsageReport};

    fn usage(reports: &[UsageReport]) -> ChatUsage {
        let mut u = ChatUsage::default();
        for &r in reports {
            u.record(r);
        }
        u
    }

    #[test]
    fn chip_label_appends_cost_for_priced_models() {
        // 100k in + 30k out under gpt-5 → $0.125 + $0.30 ≈ $0.42.
        let u = usage(&[UsageReport::tokens(100_000, 30_000)]);
        assert_eq!(chip_label(&u.meter().unwrap(), &u, "gpt-5"), "130k tok · $0.42");
    }

    #[test]
    fn chip_label_pairs_fill_percent_with_cost() {
        // 140k/200k → 70% fill; the same tokens price at ≈$0.53.
        let u = usage(&[UsageReport {
            context: Some(200_000),
            ..UsageReport::tokens(100_000, 40_000)
        }]);
        assert_eq!(chip_label(&u.meter().unwrap(), &u, "gpt-5-codex"), "70% · $0.53");
    }

    #[test]
    fn chip_label_omits_cost_for_unpriced_models() {
        // Local/free providers (ollama, sim, http, mcp) and unknown ids
        // show tokens only — no suffix, not even "$0".
        let u = usage(&[UsageReport::tokens(100_000, 30_000)]);
        for model in ["llama3.2", "sim-x", "fable", ""] {
            assert_eq!(chip_label(&u.meter().unwrap(), &u, model), "130k tok", "model {model:?}");
        }
    }

    #[test]
    fn chip_tooltip_adds_estimate_and_rate() {
        let u = usage(&[UsageReport::tokens(100_000, 30_000)]);
        let tip = chip_tooltip(&u.meter().unwrap(), &u, "gpt-5");
        assert_eq!(tip, "130,000 tokens this chat · ~$0.425 est · $1.25 in / $10 out per Mtok");
        // Unpriced models keep the bare detail line.
        let tip = chip_tooltip(&u.meter().unwrap(), &u, "llama3.2");
        assert_eq!(tip, "130,000 tokens this chat");
    }
}
