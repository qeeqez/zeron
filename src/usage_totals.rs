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
/// folded usage, and the chat's persisted messages for the daily buckets.
/// Pass an empty `messages` for backends whose usage stamps aren't token
/// counts (acp files written before the occupancy-stamp fix).
pub struct ChatUsageEntry<'a> {
    pub title: &'a str,
    pub model: &'a str,
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
/// break ties) and skips chats with no recorded usage. `by_day` holds
/// exactly [`CHART_DAYS`] slots, oldest first, ending at `today` — empty
/// days stay zero so the chart renders them as gaps.
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
mod tests {
    use chrono::TimeZone;

    use super::{CHART_DAYS, ChatUsage, ChatUsageEntry, TurnUsage, UsageTotals};
    use crate::model::{ChatMessage, MessageKind, Role, Usage};
    use crate::usage::UsageReport;

    fn entry<'a>(title: &'a str, model: &'a str, usage: &'a ChatUsage, messages: &'a [ChatMessage]) -> ChatUsageEntry<'a> {
        ChatUsageEntry { title, model, usage, messages }
    }

    /// The fixed "today" the bucketing tests anchor on.
    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 9, 16).unwrap()
    }

    /// Noon local time on `day` — midday avoids any DST-gap ambiguity.
    fn at(day: chrono::NaiveDate) -> std::time::SystemTime {
        chrono::Local.from_local_datetime(&day.and_hms_opt(12, 0, 0).unwrap()).unwrap().into()
    }

    /// An assistant message stamped with a turn's token usage.
    fn msg(day: chrono::NaiveDate, input: u64, output: u64) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            kind: MessageKind::Text("r".into()),
            rating: None,
            at: at(day),
            usage: Some(Usage { input, output }),
            attachments: vec![],
            bookmarked: false,
            alternatives: vec![],
        }
    }

    #[test]
    fn gather_sums_two_chats_and_groups_by_model() {
        let mut a = ChatUsage::default();
        a.record(UsageReport { input: 100, output: 40, cached: 60, ..UsageReport::default() });
        let mut b = ChatUsage::default();
        b.record(UsageReport::tokens(50, 10));
        let mut c = ChatUsage::default();
        c.record(UsageReport::tokens(10, 10));

        let t = UsageTotals::gather(
            &[entry("one", "gpt-5", &a, &[]), entry("two", "gpt-5", &b, &[]), entry("three", "sim-x", &c, &[])],
            today(),
        );
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
        let t = UsageTotals::gather(&[], today());
        assert_eq!(t.total_input, 0);
        assert_eq!(t.total_output, 0);
        assert_eq!(t.total_cost, 0.);
        assert!(!t.cost_partial);
        assert!(t.by_model.is_empty());
        assert!(t.by_chat.is_empty());
        assert_eq!(t.by_day.len(), CHART_DAYS, "the chart always spans the window");
        assert!(t.by_day.iter().all(|d| d.tokens == 0 && d.cost.is_none()));
        assert_eq!(t.by_day[CHART_DAYS - 1].day, today(), "the last slot is today");

        // A chat that never reported usage contributes nothing.
        let idle = ChatUsage::default();
        let t = UsageTotals::gather(&[entry("idle", "gpt-5", &idle, &[])], today());
        assert_eq!(t.total_input, 0);
        assert!(t.by_model.is_empty());
        assert!(t.by_chat.is_empty());
        assert!(t.by_day.iter().all(|d| d.tokens == 0));
    }

    #[test]
    fn gather_groups_empty_model_as_unknown() {
        let mut u = ChatUsage::default();
        u.record(UsageReport::tokens(10, 5));
        let t = UsageTotals::gather(&[entry("legacy", "", &u, &[])], today());
        assert_eq!(t.by_model.len(), 1);
        assert_eq!(t.by_model[0].model, "unknown");
        assert_eq!(t.by_model[0].cost, None, "no model id prices nothing");
        assert!(t.cost_partial);
    }

    #[test]
    fn gather_buckets_message_usage_by_local_day() {
        let today = today();
        let two_back = today - chrono::Duration::days(2);
        // Runtime usage is empty — the chart reads the persisted stamps.
        let u = ChatUsage::default();
        let msgs = vec![msg(today, 100, 40), msg(two_back, 50, 10), msg(two_back, 20, 5)];

        let t = UsageTotals::gather(&[entry("c", "gpt-5", &u, &msgs)], today);
        assert_eq!(t.by_day.len(), CHART_DAYS);
        assert_eq!(t.by_day[CHART_DAYS - 1].day, today);
        assert_eq!(t.by_day[CHART_DAYS - 1].tokens, 140);
        assert_eq!(t.by_day[CHART_DAYS - 3].day, two_back);
        assert_eq!(t.by_day[CHART_DAYS - 3].tokens, 85, "same-day events sum");
        assert!(t.by_day[CHART_DAYS - 2].tokens == 0, "the gap day stays empty");
        // gpt-5 is priced — bucketed days carry a cost estimate.
        assert!(t.by_day[CHART_DAYS - 1].cost.unwrap() > 0.);
    }

    #[test]
    fn gather_drops_events_outside_the_window() {
        let today = today();
        let u = ChatUsage::default();
        let msgs = vec![
            msg(today - chrono::Duration::days(CHART_DAYS as i64), 999, 0), // one day too old
            msg(today - chrono::Duration::days(CHART_DAYS as i64 - 1), 100, 0), // oldest kept
            msg(today + chrono::Duration::days(1), 50, 0),                  // future → today
        ];

        let t = UsageTotals::gather(&[entry("c", "sim-x", &u, &msgs)], today);
        assert_eq!(t.by_day[0].tokens, 100, "the oldest slot keeps day -13");
        assert_eq!(t.by_day[CHART_DAYS - 1].tokens, 50, "future dates clamp onto today");
        assert_eq!(t.by_day.iter().map(|d| d.tokens).sum::<u64>(), 150, "the out-of-window event is dropped");
        assert!(t.by_day.iter().all(|d| d.cost.is_none()), "sim-x prices nothing");
    }

    #[test]
    fn gather_buckets_alternatives_and_skips_unstamped() {
        let today = today();
        let u = ChatUsage::default();
        // A regenerated reply: the live message plus one older alternative,
        // each stamped with its own turn's usage — both spent tokens.
        let mut m = msg(today, 100, 40);
        m.alternatives.push(msg(today, 80, 30));
        let mut bare = msg(today, 0, 0);
        bare.usage = None;

        let t = UsageTotals::gather(&[entry("c", "gpt-5", &u, &[m, bare])], today);
        assert_eq!(t.by_day[CHART_DAYS - 1].tokens, 250, "alternatives count, unstamped messages don't");
    }
}
