//! Token-usage accumulation for a chat thread. Backends stream
//! `AgentEvent::Usage` reports; `ChatUsage` folds them into per-turn and
//! cumulative counters, a per-turn breakdown and cost estimate for the
//! usage popover, plus the context-window occupancy the meter renders.

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

/// Token split for one turn — or, folded onto `ChatUsage`, the thread's
/// cumulative split. `cached` is billed like input but priced apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

impl TurnUsage {
    /// `self` minus `earlier`, per component — `earlier` is the
    /// running-total baseline the next report's delta is measured against.
    fn delta_since(&self, e: Self) -> Self {
        Self {
            input: self.input.saturating_sub(e.input),
            output: self.output.saturating_sub(e.output),
            cached: self.cached.saturating_sub(e.cached),
        }
    }

    fn accrue(&mut self, d: Self) {
        self.input += d.input;
        self.output += d.output;
        self.cached += d.cached;
    }

    /// Total tokens across the split.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cached
    }
}

/// Usage folded onto a chat: the in-flight turn's tokens, the thread's
/// cumulative tokens, a per-turn breakdown for the usage popover, and the
/// context window's fill when a backend reports it. Runtime state — not
/// persisted.
#[derive(Clone, Debug, Default)]
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
    /// Completed turns' token splits, oldest first — the popover's rows.
    /// The in-flight turn isn't here yet; `turn_rows` appends it.
    turns: Vec<TurnUsage>,
    /// The in-flight turn's split — becomes a `turns` row on `begin_turn`.
    turn_tokens: TurnUsage,
    /// The thread's cumulative split — what `cost` prices.
    tokens: TurnUsage,
    /// Last token report — reports are running totals for the turn, so the
    /// delta since `last` is what accrues.
    last: TurnUsage,
    /// Latest rate-limit/quota snapshot — the chat banner reads `limited`,
    /// the popover reads the windows. Runtime state, not persisted.
    pub rate_limit: Option<crate::rate_limit::RateLimit>,
}

impl ChatUsage {
    /// A new turn starts: the completed turn's split joins `turns` and the
    /// per-turn counters and report baseline reset.
    pub fn begin_turn(&mut self) {
        if self.turn_tokens.total() > 0 {
            self.turns.push(self.turn_tokens);
        }
        self.turn = 0;
        self.turn_tokens = TurnUsage::default();
        self.last = TurnUsage::default();
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
        let sum = TurnUsage {
            input: report.input,
            output: report.output,
            cached: report.cached,
        };
        let delta = sum.delta_since(self.last);
        self.last = sum;
        self.turn += delta.total();
        self.total += delta.total();
        self.turn_tokens.accrue(delta);
        self.tokens.accrue(delta);
    }

    /// Fold a rate-limit signal in. A bare flag (error-derived, no
    /// windows) merges onto the current snapshot so quota rows survive;
    /// a snapshot with windows replaces it wholesale.
    pub fn record_rate_limit(&mut self, rl: crate::rate_limit::RateLimit) {
        if rl.primary.is_none()
            && rl.secondary.is_none()
            && let Some(cur) = &mut self.rate_limit
        {
            cur.limited = rl.limited;
            cur.reset_hint = rl.reset_hint.or_else(|| cur.reset_hint.take());
            return;
        }
        self.rate_limit = Some(rl);
    }

    /// A turn completed without an error — the limit cleared. Quota
    /// windows stay: the popover still shows how full they are.
    pub fn clear_limited(&mut self) {
        if let Some(rl) = &mut self.rate_limit {
            rl.limited = false;
            rl.reset_hint = None;
        }
    }

    /// Completed turns plus the in-flight one when it has tokens — the
    /// popover's per-turn rows, oldest first.
    pub fn turn_rows(&self) -> Vec<TurnUsage> {
        let mut rows = self.turns.clone();
        if self.turn_tokens.total() > 0 {
            rows.push(self.turn_tokens);
        }
        rows
    }

    /// The thread's cumulative token split — the chat info dialog's
    /// in/out/cached row. `usage_totals` reads the field directly (it's a
    /// submodule); views go through this.
    pub fn tokens(&self) -> TurnUsage {
        self.tokens
    }

    /// Estimated USD cost of the thread's cumulative tokens under the
    /// model's pricing — `None` when the model's rates are unknown, so the
    /// UI shows tokens only rather than a made-up number.
    pub fn cost(&self, model: &str) -> Option<f64> {
        crate::pricing::model_pricing(model).map(|p| p.cost(self.tokens))
    }

    /// Context-window fill as a 0..=1 fraction — `None` when no backend
    /// has reported a window size. The titlebar chip reads `meter()`.
    pub fn fill(&self) -> Option<f32> {
        match self.meter() {
            Some(ContextMeter::Fill { fill, .. }) => Some(fill),
            _ => None,
        }
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

/// Session-wide usage folded across chats — the popover's bottom row.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionUsage {
    /// Cumulative tokens across every chat in the window.
    pub total: u64,
    /// Sum of the priced chats' estimated cost.
    pub cost: f64,
    /// Some chat has tokens but no known pricing — `cost` is a lower bound.
    pub cost_partial: bool,
}

/// The titlebar chip's meter state — a submodule so this file stays under
/// the SLOC cap. Re-exported: callers keep using `crate::usage::*`.
#[path = "usage_meter.rs"]
pub(crate) mod meter;
pub use meter::{ContextMeter, MeterTier};

/// Workspace-wide aggregation for the usage panel — a submodule so this
/// file stays under the SLOC cap. Re-exported: callers keep using
/// `crate::usage::UsageTotals`.
#[path = "usage_totals.rs"]
pub(crate) mod totals;
pub use totals::{BreakdownRow, ChatTotal, ChatUsageEntry, DayTotal, UsageTotals};
/// `1234` → `1.2k`, `12600` → `13k`, `1_500_000` → `1.5M`.
pub(crate) fn fmt_tokens(n: u64) -> String {
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
    use super::{ChatUsage, TurnUsage, UsageReport, fmt_tokens};

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
    fn turns_collect_for_the_popover() {
        let mut u = ChatUsage::default();
        u.begin_turn(); // first send — nothing to collect yet
        u.record(UsageReport::tokens(100, 40));
        u.begin_turn();
        u.record(UsageReport::tokens(50, 10));
        let rows = u.turn_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], TurnUsage { input: 100, output: 40, cached: 0 });
        assert_eq!(rows[1], TurnUsage { input: 50, output: 10, cached: 0 });
    }

    #[test]
    fn cost_computes_from_the_pricing_table() {
        let mut u = ChatUsage::default();
        u.record(UsageReport {
            input: 1_000_000,
            output: 500_000,
            cached: 0,
            ..UsageReport::default()
        });
        // gpt-5: $1.25/1M in, $10/1M out → 1.25 + 5.00.
        assert_eq!(u.cost("gpt-5"), Some(6.25));
        // Aliases match by substring.
        assert!(u.cost("claude-sonnet-4-5").is_some());
        assert!(u.cost("sonnet").is_some());
    }

    #[test]
    fn cost_is_none_for_unknown_models() {
        let mut u = ChatUsage::default();
        u.record(UsageReport::tokens(100, 40));
        assert_eq!(u.cost("fable"), None);
        assert_eq!(u.cost("sim-x"), None);
        assert_eq!(u.cost(""), None);
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
        assert!(u.turn_rows().is_empty(), "occupancy isn't a token turn");
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
