//! Workspace-wide usage aggregation for the usage dashboard — the
//! per-model table, per-chat ranking, and the last-two-weeks daily chart.
//! Headline totals fold from every chat's `ChatUsage`; the chart buckets
//! the *persisted* per-message `usage` stamps by local day so history
//! survives restarts (`ChatUsage` itself is runtime-only). A submodule of
//! `crate::usage` (declared there via `#[path]` so `usage.rs` stays under
//! the SLOC cap); the parent re-exports these types, so callers use
//! `crate::usage::UsageTotals`.

use super::{ChatUsage, TurnUsage};

/// One chat's contribution to [`UsageTotals::gather`]: the display title,
/// the model its tokens are priced under (callers apply their fallback —
/// the workspace substitutes the current selection for legacy chats), the
/// provider's display label, the folded usage, and the chat's persisted
/// messages for the daily buckets. Pass an empty `messages` for backends
/// whose usage stamps aren't token counts (acp files written before the
/// occupancy-stamp fix).
pub struct ChatUsageEntry<'a> {
    pub title: &'a str,
    pub model: &'a str,
    /// The provider instance's user-facing name — the breakdown's row
    /// label; callers resolve the id to a name (deleted instances fall
    /// back to the id, empty to "unknown").
    pub provider: &'a str,
    pub usage: &'a ChatUsage,
    pub messages: &'a [crate::model::ChatMessage],
}

/// Days the dashboard chart spans, ending today.
pub const CHART_DAYS: usize = 14;

/// One day's bar: total tokens across every chat's messages that day, and
/// the summed estimate — `None` when no event that day priced (a partially
/// priced day still reports the priced share, like `total_cost`).
#[derive(Clone, Debug)]
pub struct DayTotal {
    pub day: chrono::NaiveDate,
    pub tokens: u64,
    pub cost: Option<f64>,
}

/// One row of a breakdown table — per-model and per-provider share the
/// shape: the folded token split and the summed estimate, `None` when the
/// row's pricing is unknown.
#[derive(Clone, Debug)]
pub struct BreakdownRow {
    pub label: String,
    pub tokens: TurnUsage,
    pub cost: Option<f64>,
}

/// One row of the per-chat ranking — `cost` is `None` when the chat's
/// model is unpriced; `tokens` backs the tie-break and the row's token
/// column.
#[derive(Clone, Debug)]
pub struct ChatTotal {
    pub title: String,
    pub tokens: u64,
    pub cost: Option<f64>,
}

/// Workspace-wide usage folded across every chat — the usage panel's
/// data. `total_input` counts cache tokens on the input side (they're
/// billed like input); `total_cost` sums only the priced chats, so
/// `cost_partial` marks it a lower bound. `by_model` and `by_provider`
/// sort by tokens descending; `by_chat` by cost descending (unpriced
/// chats last, tokens break ties) and skips chats with no recorded usage.
/// `by_day` holds exactly [`CHART_DAYS`] slots, oldest first, ending at
/// `today` — empty days stay zero so the chart renders them as gaps.
#[derive(Clone, Debug, Default)]
pub struct UsageTotals {
    pub total_input: u64,
    pub total_output: u64,
    pub total_cost: f64,
    /// Some chat has tokens but no known pricing — `total_cost` is a
    /// lower bound.
    pub cost_partial: bool,
    pub by_provider: Vec<BreakdownRow>,
    pub by_model: Vec<BreakdownRow>,
    pub by_chat: Vec<ChatTotal>,
    pub by_day: Vec<DayTotal>,
}

impl UsageTotals {
    /// Fold every chat's usage into the dashboard's totals. Pure over the
    /// entries — pricing comes from `crate::pricing`, never recomputed.
    /// `today` anchors the daily buckets (callers pass the local date).
    pub fn gather(chats: &[ChatUsageEntry<'_>], today: chrono::NaiveDate) -> Self {
        let mut totals = Self {
            by_day: (0..CHART_DAYS)
                .map(|i| DayTotal {
                    day: today - chrono::Duration::days((CHART_DAYS - 1 - i) as i64),
                    tokens: 0,
                    cost: None,
                })
                .collect(),
            ..Self::default()
        };
        for entry in chats {
            // The chart reads the persisted stamps even when this window's
            // runtime counters are empty (a chat loaded from disk).
            bucket_days(&mut totals.by_day, entry, today);
            let tokens = entry.usage.tokens;
            if tokens.total() == 0 {
                continue;
            }
            totals.total_input += tokens.input + tokens.cached;
            totals.total_output += tokens.output;
            let cost = entry.usage.cost(entry.model);
            match cost {
                Some(c) => totals.total_cost += c,
                None => totals.cost_partial = true,
            }
            let model = if entry.model.is_empty() { "unknown" } else { entry.model };
            let row = breakdown_row(&mut totals.by_model, model);
            row.tokens.accrue(tokens);
            if let Some(c) = cost {
                row.cost = Some(row.cost.unwrap_or(0.) + c);
            }
            let provider = if entry.provider.is_empty() { "unknown" } else { entry.provider };
            let row = breakdown_row(&mut totals.by_provider, provider);
            row.tokens.accrue(tokens);
            if let Some(c) = cost {
                row.cost = Some(row.cost.unwrap_or(0.) + c);
            }
            totals.by_chat.push(ChatTotal { title: entry.title.to_string(), tokens: tokens.total(), cost });
        }
        totals
            .by_model
            .sort_by(|a, b| b.tokens.total().cmp(&a.tokens.total()).then_with(|| a.label.cmp(&b.label)));
        totals
            .by_provider
            .sort_by(|a, b| b.tokens.total().cmp(&a.tokens.total()).then_with(|| a.label.cmp(&b.label)));
        totals
            .by_chat
            .sort_by(|a, b| b.cost.unwrap_or(0.).total_cmp(&a.cost.unwrap_or(0.)).then_with(|| b.tokens.cmp(&a.tokens)));
        totals
    }
}

/// The `label` row in `rows`, creating it when absent — the breakdown
/// tables' find-or-push.
fn breakdown_row<'a>(rows: &'a mut Vec<BreakdownRow>, label: &str) -> &'a mut BreakdownRow {
    if let Some(ix) = rows.iter().position(|r| r.label == label) {
        return &mut rows[ix];
    }
    rows.push(BreakdownRow {
        label: label.to_string(),
        tokens: TurnUsage::default(),
        cost: None,
    });
    rows.last_mut().expect("just pushed")
}

/// Fold one chat's persisted message usage into the daily buckets. Each
/// turn's token total is stamped on its last assistant message (and on
/// each regenerated alternative — those turns spent tokens too), dated by
/// the message's own `at` in local time. Days outside the window drop;
/// future-dated messages clamp onto today.
fn bucket_days(by_day: &mut [DayTotal], entry: &ChatUsageEntry<'_>, today: chrono::NaiveDate) {
    let pricing = crate::pricing::model_pricing(entry.model);
    for msg in entry.messages.iter().flat_map(|m| m.alternatives.iter().chain(std::iter::once(m))) {
        let Some(u) = msg.usage else { continue };
        let day = chrono::DateTime::<chrono::Local>::from(msg.at).date_naive();
        let age = today.signed_duration_since(day).num_days();
        if age >= CHART_DAYS as i64 {
            continue;
        }
        let slot = &mut by_day[CHART_DAYS - 1 - age.max(0) as usize];
        slot.tokens += u.input + u.output;
        if let Some(p) = pricing {
            let c = p.cost(TurnUsage { input: u.input, output: u.output, cached: 0 });
            slot.cost = Some(slot.cost.unwrap_or(0.) + c);
        }
    }
}

#[cfg(test)]
#[path = "usage_totals_tests.rs"]
mod tests;
