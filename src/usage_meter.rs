//! The titlebar's context-window meter — what the chip shows, derived
//! from `ChatUsage`. acp's `usage_update` reports real occupancy (used of
//! size) → a `NN%` fill; token backends (codex, claude, http, ollama)
//! never report a window size, so the chip falls back to the chat's
//! cumulative token count; no reports at all (fresh chat, sim) → hidden.
//! `ChatUsage` is runtime state, so a reloaded chat shows the meter again
//! only after the next report.

use super::ChatUsage;

/// Fill severity — the chip's text color. Fractions of the window:
/// warning at 80%+, danger at 95%+.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterTier {
    Calm,
    Warning,
    Danger,
}

impl MeterTier {
    fn of(fill: f32) -> Self {
        if fill >= 0.95 {
            Self::Danger
        } else if fill >= 0.80 {
            Self::Warning
        } else {
            Self::Calm
        }
    }
}

/// What the titlebar chip renders. `Fill` is real occupancy — each report
/// overwrites `context_used`, so it tracks the latest assistant turn.
/// `Tokens` is the fallback when no window size is known: cumulative
/// tokens can't be a percent, so the chip shows the count and stays
/// muted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextMeter {
    Fill { used: u64, size: u64, fill: f32, tier: MeterTier },
    Tokens { total: u64 },
}

impl ContextMeter {
    /// The chip's visible text: `NN%` of the window used, or `{n} tok`
    /// for the token fallback.
    pub fn label(&self) -> String {
        match *self {
            Self::Fill { fill, .. } => format!("{}%", (fill * 100.).round() as u32),
            Self::Tokens { total } => format!("{} tok", super::fmt_tokens(total)),
        }
    }

    /// Tooltip + aria label — exact counts, not the compact label.
    pub fn detail(&self) -> String {
        match *self {
            Self::Fill { used, size, .. } => format!("{} / {} tokens", commas(used), commas(size)),
            Self::Tokens { total } => format!("{} tokens this chat", commas(total)),
        }
    }
}

impl ChatUsage {
    /// The titlebar meter's state — `None` until the first usage report.
    pub fn meter(&self) -> Option<ContextMeter> {
        if let Some(size) = self.context {
            let used = self.context_used.unwrap_or(self.total);
            let fill = (used as f32 / size as f32).clamp(0., 1.);
            return Some(ContextMeter::Fill { used, size, fill, tier: MeterTier::of(fill) });
        }
        (self.total > 0).then_some(ContextMeter::Tokens { total: self.total })
    }
}

/// `12340` → `"12,340"` — the tooltip's exact counts.
fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ContextMeter, MeterTier};
    use crate::usage::{ChatUsage, UsageReport};

    fn usage(reports: &[UsageReport]) -> ChatUsage {
        let mut u = ChatUsage::default();
        for &r in reports {
            u.record(r);
        }
        u
    }

    #[test]
    fn meter_stays_hidden_without_reports() {
        assert_eq!(ChatUsage::default().meter(), None);
    }

    #[test]
    fn token_reports_fall_back_to_cumulative_count() {
        // codex/claude report tokens only — no window size, so no percent.
        // Reports are the turn's running total: 140 then 450 accrues 450.
        let m = usage(&[UsageReport::tokens(100, 40), UsageReport::tokens(300, 150)]).meter();
        assert_eq!(m, Some(ContextMeter::Tokens { total: 450 }));
        assert_eq!(m.unwrap().label(), "450 tok");
        assert_eq!(m.unwrap().detail(), "450 tokens this chat");
    }

    #[test]
    fn occupancy_reports_fill_and_latest_wins() {
        let m = usage(&[UsageReport::occupancy(1_200, 200_000), UsageReport::occupancy(170_000, 200_000)]).meter();
        let Some(ContextMeter::Fill { used, size, fill, tier }) = m else { panic!("expected Fill") };
        assert_eq!((used, size), (170_000, 200_000), "the latest report overwrites the earlier one");
        assert!((fill - 0.85).abs() < 0.001);
        assert_eq!(tier, MeterTier::Warning);
        assert_eq!(m.unwrap().label(), "85%");
        assert_eq!(m.unwrap().detail(), "170,000 / 200,000 tokens");
    }

    #[test]
    fn tier_thresholds() {
        let tier = |used: u64| match usage(&[UsageReport::occupancy(used, 200_000)]).meter() {
            Some(ContextMeter::Fill { tier, .. }) => tier,
            m => panic!("expected Fill, got {m:?}"),
        };
        assert_eq!(tier(0), MeterTier::Calm);
        assert_eq!(tier(159_999), MeterTier::Calm, "just under 80%");
        assert_eq!(tier(160_000), MeterTier::Warning, "80% warns");
        assert_eq!(tier(189_999), MeterTier::Warning, "just under 95%");
        assert_eq!(tier(190_000), MeterTier::Danger, "95% is danger");
        assert_eq!(tier(250_000), MeterTier::Danger, "over-full clamps");
    }

    #[test]
    fn over_full_fill_clamps_to_100_percent() {
        let m = usage(&[UsageReport::occupancy(250_000, 200_000)]).meter().unwrap();
        assert_eq!(m.label(), "100%");
    }

    #[test]
    fn commas_groups_thousands() {
        assert_eq!(super::commas(0), "0");
        assert_eq!(super::commas(999), "999");
        assert_eq!(super::commas(1_234), "1,234");
        assert_eq!(super::commas(1_234_567), "1,234,567");
    }
}
