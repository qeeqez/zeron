//! Per-model pricing for the usage popover's cost estimate. Rates are the
//! providers' published API prices in USD per million tokens; a model with
//! no row returns `None` so the UI shows tokens only rather than guessing.

use crate::usage::TurnUsage;

/// Per-million-token USD rates for a known model. `cached` is the
/// cache-read rate — providers bill it below plain input.
#[derive(Clone, Copy, Debug)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    pub cached: f64,
}

impl ModelPricing {
    /// USD cost of a token split under these rates.
    pub fn cost(&self, t: TurnUsage) -> f64 {
        (t.input as f64 * self.input + t.output as f64 * self.output + t.cached as f64 * self.cached) / 1_000_000.
    }
}

/// Published API rates, $/1M tokens: (substring, input, output, cached).
/// Aliases match by substring — "sonnet" prices claude-sonnet-* — so the
/// table is ordered most-specific first ("gpt-5-mini" before "gpt-5").
/// Models not listed (fable, sim, http, acp) return `None`.
const PRICING: &[(&str, f64, f64, f64)] = &[
    ("gpt-5-codex", 1.25, 10.00, 0.125),
    ("gpt-5-mini", 0.25, 2.00, 0.025),
    ("gpt-5", 1.25, 10.00, 0.125),
    ("codex-mini", 1.50, 6.00, 0.375),
    ("opus", 5.00, 25.00, 0.50),
    ("sonnet", 3.00, 15.00, 0.30),
    ("haiku", 1.00, 5.00, 0.10),
];

/// The pricing row for a model id or alias — `None` when unknown.
pub fn model_pricing(model: &str) -> Option<ModelPricing> {
    let m = model.to_lowercase();
    PRICING
        .iter()
        .find(|(name, ..)| m.contains(name))
        .map(|&(_, input, output, cached)| ModelPricing { input, output, cached })
}

/// USD estimate for the popover/footer: `$1.23`, `$0.0234`, `$0.000125`.
/// Six decimals keep sub-cent turns visible; trailing zeros trim away.
pub(crate) fn fmt_cost(usd: f64) -> String {
    let s = format!("${usd:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::{fmt_cost, model_pricing};

    #[test]
    fn pricing_prefers_the_specific_row() {
        // "gpt-5-mini" contains "gpt-5" — the mini rates must win.
        assert_eq!(model_pricing("gpt-5-mini").unwrap().input, 0.25);
        assert_eq!(model_pricing("GPT-5-Codex").unwrap().input, 1.25);
        // Aliases match by substring.
        assert!(model_pricing("claude-sonnet-4-5").is_some());
        assert!(model_pricing("sonnet").is_some());
        // Unknown models price nothing.
        assert!(model_pricing("fable").is_none());
        assert!(model_pricing("sim-x").is_none());
        assert!(model_pricing("").is_none());
    }

    #[test]
    fn fmt_cost_trims_to_significant_cents() {
        assert_eq!(fmt_cost(0.), "$0");
        assert_eq!(fmt_cost(6.25), "$6.25");
        assert_eq!(fmt_cost(0.0234), "$0.0234");
        assert_eq!(fmt_cost(0.000125), "$0.000125");
    }
}
