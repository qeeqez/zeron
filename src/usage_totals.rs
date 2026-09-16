//! Workspace-wide usage aggregation for the usage dashboard — the
//! per-model table and per-chat ranking folded from every chat's
//! `ChatUsage`. A submodule of `crate::usage` (declared there via
//! `#[path]` so `usage.rs` stays under the SLOC cap); the parent
//! re-exports these types, so callers use `crate::usage::UsageTotals`.

use super::{ChatUsage, TurnUsage};

/// One chat's contribution to [`UsageTotals::gather`]: the display title,
/// the model its tokens are priced under (callers apply their fallback —
/// the workspace substitutes the current selection for legacy chats), and
/// the folded usage.
pub struct ChatUsageEntry<'a> {
    pub title: &'a str,
    pub model: &'a str,
    pub usage: &'a ChatUsage,
}

/// One row of the per-model table: the folded token split and the summed
/// estimate — `None` when the model's pricing is unknown.
#[derive(Clone, Debug)]
pub struct ModelTotal {
    pub model: String,
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

/// Workspace-wide usage folded across every chat — the usage dashboard's
/// data. `total_input` counts cache tokens on the input side (they're
/// billed like input); `total_cost` sums only the priced chats, so
/// `cost_partial` marks it a lower bound. `by_model` sorts by tokens
/// descending; `by_chat` by cost descending (unpriced chats last, tokens
/// break ties) and skips chats with no recorded usage.
#[derive(Clone, Debug, Default)]
pub struct UsageTotals {
    pub total_input: u64,
    pub total_output: u64,
    pub total_cost: f64,
    /// Some chat has tokens but no known pricing — `total_cost` is a
    /// lower bound.
    pub cost_partial: bool,
    pub by_model: Vec<ModelTotal>,
    pub by_chat: Vec<ChatTotal>,
}

impl UsageTotals {
    /// Fold every chat's usage into the dashboard's totals. Pure over the
    /// entries — pricing comes from `crate::pricing`, never recomputed.
    pub fn gather(chats: &[ChatUsageEntry<'_>]) -> Self {
        let mut totals = Self::default();
        for entry in chats {
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
            let row = match totals.by_model.iter_mut().find(|m| m.model == model) {
                Some(m) => m,
                None => {
                    totals.by_model.push(ModelTotal {
                        model: model.to_string(),
                        tokens: TurnUsage::default(),
                        cost: None,
                    });
                    totals.by_model.last_mut().expect("just pushed")
                },
            };
            row.tokens.accrue(tokens);
            if let Some(c) = cost {
                row.cost = Some(row.cost.unwrap_or(0.) + c);
            }
            totals.by_chat.push(ChatTotal { title: entry.title.to_string(), tokens: tokens.total(), cost });
        }
        totals
            .by_model
            .sort_by(|a, b| b.tokens.total().cmp(&a.tokens.total()).then_with(|| a.model.cmp(&b.model)));
        totals
            .by_chat
            .sort_by(|a, b| b.cost.unwrap_or(0.).total_cmp(&a.cost.unwrap_or(0.)).then_with(|| b.tokens.cmp(&a.tokens)));
        totals
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatUsage, ChatUsageEntry, TurnUsage, UsageTotals};
    use crate::usage::UsageReport;

    fn entry<'a>(title: &'a str, model: &'a str, usage: &'a ChatUsage) -> ChatUsageEntry<'a> {
        ChatUsageEntry { title, model, usage }
    }

    #[test]
    fn gather_sums_two_chats_and_groups_by_model() {
        let mut a = ChatUsage::default();
        a.record(UsageReport { input: 100, output: 40, cached: 60, ..UsageReport::default() });
        let mut b = ChatUsage::default();
        b.record(UsageReport::tokens(50, 10));
        let mut c = ChatUsage::default();
        c.record(UsageReport::tokens(10, 10));

        let t = UsageTotals::gather(&[entry("one", "gpt-5", &a), entry("two", "gpt-5", &b), entry("three", "sim-x", &c)]);
        // Cache tokens count on the input side.
        assert_eq!(t.total_input, 100 + 60 + 50 + 10);
        assert_eq!(t.total_output, 40 + 10 + 10);
        assert!(t.cost_partial, "sim-x has no pricing — the sum is a lower bound");
        assert!(t.total_cost > 0., "the priced chats still contribute");

        assert_eq!(t.by_model.len(), 2, "the two gpt-5 chats fold into one row");
        let gpt5 = &t.by_model[0];
        assert_eq!(gpt5.model, "gpt-5");
        assert_eq!(gpt5.tokens, TurnUsage { input: 150, output: 50, cached: 60 });
        assert_eq!(gpt5.cost, Some(t.total_cost), "only gpt-5 is priced");
        assert_eq!(t.by_model[1].model, "sim-x");
        assert_eq!(t.by_model[1].cost, None);

        // Priced chats rank ahead of the unpriced one.
        assert_eq!(t.by_chat.len(), 3);
        assert_eq!(t.by_chat[2].title, "three");
        assert_eq!(t.by_chat[2].cost, None);
    }

    #[test]
    fn gather_empty_and_unused_chats_yield_zeros() {
        let t = UsageTotals::gather(&[]);
        assert_eq!(t.total_input, 0);
        assert_eq!(t.total_output, 0);
        assert_eq!(t.total_cost, 0.);
        assert!(!t.cost_partial);
        assert!(t.by_model.is_empty());
        assert!(t.by_chat.is_empty());

        // A chat that never reported usage contributes nothing.
        let idle = ChatUsage::default();
        let t = UsageTotals::gather(&[entry("idle", "gpt-5", &idle)]);
        assert_eq!(t.total_input, 0);
        assert!(t.by_model.is_empty());
        assert!(t.by_chat.is_empty());
    }

    #[test]
    fn gather_groups_empty_model_as_unknown() {
        let mut u = ChatUsage::default();
        u.record(UsageReport::tokens(10, 5));
        let t = UsageTotals::gather(&[entry("legacy", "", &u)]);
        assert_eq!(t.by_model.len(), 1);
        assert_eq!(t.by_model[0].model, "unknown");
        assert_eq!(t.by_model[0].cost, None, "no model id prices nothing");
        assert!(t.cost_partial);
    }
}
