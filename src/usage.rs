//! Token-usage accumulation for a chat thread. Backends stream
//! `AgentEvent::Usage` reports; `ChatUsage` folds them into per-turn and
//! cumulative counters plus the context-window occupancy the composer
//! meter renders.

/// One backend usage report. Token backends fill `input`/`output`/`cached`;
/// occupancy backends (acp's `usage_update`) fill `context_used`/`context`
/// instead — the report then carries no token counts.
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageReport {
    pub input: u64,
    pub output: u64,
    /// Cache creation/read tokens — billed like input, tracked apart.
    pub cached: u64,
    /// Tokens currently occupying the context window, when reported.
    pub context_used: Option<u64>,
    /// The model's context-window size, when reported.
    pub context: Option<u64>,
}

impl UsageReport {
    /// A token-count report (codex, claude).
    pub fn tokens(input: u64, output: u64) -> Self {
        Self { input, output, ..Self::default() }
    }

    /// A context-occupancy report (acp `usage_update`: used of size).
    pub fn occupancy(used: u64, size: u64) -> Self {
        Self {
            context_used: Some(used),
            context: Some(size),
            ..Self::default()
        }
    }
}

/// Usage folded onto a chat: the in-flight turn's tokens, the thread's
/// cumulative tokens, and the context window's fill when a backend reports
/// it. Runtime state — not persisted.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChatUsage {
    /// Tokens used by the current (or last) turn.
    pub turn: u64,
    /// Cumulative tokens across the thread's turns.
    pub total: u64,
    /// Context-window size reported by the backend.
    pub context: Option<u64>,
    /// Context tokens in use — explicit when reported, else `total` stands
    /// in as the fill numerator.
    pub context_used: Option<u64>,
    /// Sum of the last token report — reports are running totals for the
    /// turn, so the delta since `last` is what accrues.
    last: u64,
}

impl ChatUsage {
    /// A new turn starts: the per-turn counter and report baseline reset.
    pub fn begin_turn(&mut self) {
        self.turn = 0;
        self.last = 0;
    }

    /// Fold one report in. Token reports arrive as the turn's running
    /// total (codex `last`, claude cumulative), so only the delta since
    /// the previous report accrues — repeated reports can't double-count.
    /// Occupancy reports carry no tokens and only move the context meter.
    pub fn record(&mut self, report: UsageReport) {
        if let Some(size) = report.context.filter(|s| *s > 0) {
            self.context = Some(size);
        }
        if let Some(used) = report.context_used {
            self.context_used = Some(used);
            return;
        }
        let sum = report.input + report.output + report.cached;
        let delta = sum.saturating_sub(self.last);
        self.last = sum;
        self.turn += delta;
        self.total += delta;
    }

    /// Context-window fill as a 0..=1 fraction — `None` when no backend
    /// has reported a window size.
    pub fn fill(&self) -> Option<f32> {
        let size = self.context?;
        let used = self.context_used.unwrap_or(self.total);
        Some((used as f32 / size as f32).clamp(0., 1.))
    }

    /// Compact meter text: `+{turn} · {used} / {limit}` with a known
    /// window, `+{turn} · {total} tok` without. `None` until the first
    /// report — the composer hides the meter entirely before that.
    pub fn label(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.turn > 0 {
            parts.push(format!("+{}", fmt_tokens(self.turn)));
        }
        match self.context {
            Some(size) => parts.push(format!("{} / {}", fmt_tokens(self.context_used.unwrap_or(self.total)), fmt_tokens(size))),
            None if self.total > 0 => parts.push(format!("{} tok", fmt_tokens(self.total))),
            None => {},
        }
        if parts.is_empty() { None } else { Some(parts.join(" · ")) }
    }
}

/// `1234` → `1.2k`, `12600` → `13k`, `1_500_000` → `1.5M`.
fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.)
    } else if n >= 10_000 {
        format!("{}k", (n as f64 / 1000.).round() as u64)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatUsage, UsageReport, fmt_tokens};

    #[test]
    fn token_reports_accumulate_turn_and_total() {
        let mut u = ChatUsage::default();
        // Reports are the turn's running total — deltas accrue, not sums.
        u.record(UsageReport::tokens(100, 40));
        u.record(UsageReport::tokens(300, 150));
        assert_eq!(u.turn, 450);
        assert_eq!(u.total, 450);

        u.begin_turn();
        u.record(UsageReport::tokens(50, 0));
        assert_eq!(u.turn, 50);
        assert_eq!(u.total, 500);
    }

    #[test]
    fn cached_counts_toward_the_sum() {
        let mut u = ChatUsage::default();
        u.record(UsageReport { input: 100, output: 40, cached: 60, ..UsageReport::default() });
        assert_eq!(u.turn, 200);
        assert_eq!(u.total, 200);
    }

    #[test]
    fn context_size_tracked_and_fill_computed() {
        let mut u = ChatUsage::default();
        u.record(UsageReport { context: Some(200_000), ..UsageReport::tokens(100, 40) });
        assert_eq!(u.context, Some(200_000));
        // No explicit occupancy — cumulative tokens stand in as the fill.
        assert_eq!(u.fill(), Some(140. / 200_000.));
        assert_eq!(u.label().as_deref(), Some("+140 · 140 / 200k"));
    }

    #[test]
    fn occupancy_reports_move_the_meter_not_the_counters() {
        let mut u = ChatUsage::default();
        u.record(UsageReport::occupancy(1_200, 200_000));
        assert_eq!(u.turn, 0);
        assert_eq!(u.total, 0);
        assert_eq!(u.context, Some(200_000));
        assert_eq!(u.context_used, Some(1_200));
        assert_eq!(u.label().as_deref(), Some("1.2k / 200k"));
    }

    #[test]
    fn missing_context_degrades_to_token_count() {
        let mut u = ChatUsage::default();
        assert_eq!(u.label(), None);
        u.record(UsageReport::tokens(100, 40));
        assert_eq!(u.fill(), None);
        assert_eq!(u.label().as_deref(), Some("+140 · 140 tok"));
    }

    #[test]
    fn fmt_tokens_scales() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_234), "1.2k");
        assert_eq!(fmt_tokens(12_600), "13k");
        assert_eq!(fmt_tokens(200_000), "200k");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
    }
}
