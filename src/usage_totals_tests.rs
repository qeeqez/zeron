//! Tests for `UsageTotals::gather` — the fold math, the per-provider
//! and per-model groupings, and the daily bucketing. Split from
//! `usage_totals.rs` to stay under the SLOC cap.

use chrono::TimeZone;

use super::{CHART_DAYS, ChatUsage, ChatUsageEntry, TurnUsage, UsageTotals};
use crate::model::{ChatMessage, MessageKind, Role, Usage};
use crate::usage::UsageReport;

fn entry<'a>(title: &'a str, model: &'a str, usage: &'a ChatUsage, messages: &'a [ChatMessage]) -> ChatUsageEntry<'a> {
    ChatUsageEntry { title, model, provider: "Codex", usage, messages }
}

/// Same, with an explicit provider label — the by-provider tests.
fn entry_p<'a>(title: &'a str, provider: &'a str, model: &'a str, usage: &'a ChatUsage, messages: &'a [ChatMessage]) -> ChatUsageEntry<'a> {
    ChatUsageEntry { title, model, provider, usage, messages }
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
        pinned: false,
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

    let t =
        UsageTotals::gather(&[entry("one", "gpt-5", &a, &[]), entry("two", "gpt-5", &b, &[]), entry("three", "sim-x", &c, &[])], today());
    // Cache tokens count on the input side.
    assert_eq!(t.total_input, 100 + 60 + 50 + 10);
    assert_eq!(t.total_output, 40 + 10 + 10);
    assert!(t.cost_partial, "sim-x has no pricing — the sum is a lower bound");
    assert!(t.total_cost > 0., "the priced chats still contribute");

    assert_eq!(t.by_model.len(), 2, "the two gpt-5 chats fold into one row");
    let gpt5 = &t.by_model[0];
    assert_eq!(gpt5.label, "gpt-5");
    assert_eq!(gpt5.tokens, TurnUsage { input: 150, output: 50, cached: 60 });
    assert_eq!(gpt5.cost, Some(t.total_cost), "only gpt-5 is priced");
    assert_eq!(t.by_model[1].label, "sim-x");
    assert_eq!(t.by_model[1].cost, None);

    // All three chats share the helper's "Codex" provider — one row.
    assert_eq!(t.by_provider.len(), 1);
    assert_eq!(t.by_provider[0].label, "Codex");
    assert_eq!(t.by_provider[0].tokens.total(), 280);
    assert_eq!(t.by_provider[0].cost, Some(t.total_cost));

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
    assert!(t.by_provider.is_empty());
    assert!(t.by_chat.is_empty());
    assert_eq!(t.by_day.len(), CHART_DAYS, "the chart always spans the window");
    assert!(t.by_day.iter().all(|d| d.tokens == 0 && d.cost.is_none()));
    assert_eq!(t.by_day[CHART_DAYS - 1].day, today(), "the last slot is today");

    // A chat that never reported usage contributes nothing.
    let idle = ChatUsage::default();
    let t = UsageTotals::gather(&[entry("idle", "gpt-5", &idle, &[])], today());
    assert_eq!(t.total_input, 0);
    assert!(t.by_model.is_empty());
    assert!(t.by_provider.is_empty());
    assert!(t.by_chat.is_empty());
    assert!(t.by_day.iter().all(|d| d.tokens == 0));
}

#[test]
fn gather_groups_by_provider_label() {
    let mut a = ChatUsage::default();
    a.record(UsageReport::tokens(100, 40));
    let mut b = ChatUsage::default();
    b.record(UsageReport::tokens(50, 10));
    let mut c = ChatUsage::default();
    c.record(UsageReport::tokens(10, 10));

    let t = UsageTotals::gather(
        &[
            entry_p("one", "Codex", "gpt-5", &a, &[]),
            entry_p("two", "Claude", "sonnet", &b, &[]),
            // A second Codex chat folds into the same provider row;
            // its unpriced model counts tokens but no cost.
            entry_p("three", "Codex", "sim-x", &c, &[]),
        ],
        today(),
    );
    assert_eq!(t.by_provider.len(), 2, "one row per provider label");
    let codex = &t.by_provider[0];
    assert_eq!(codex.label, "Codex");
    assert_eq!(codex.tokens, TurnUsage { input: 110, output: 50, cached: 0 });
    assert_eq!(codex.cost, t.by_model[0].cost, "only the gpt-5 chat priced");
    let claude = &t.by_provider[1];
    assert_eq!(claude.label, "Claude");
    assert_eq!(claude.tokens, TurnUsage { input: 50, output: 10, cached: 0 });
    assert!(claude.cost.is_some(), "sonnet is priced");

    // An empty provider label groups as "unknown", like the model.
    let t = UsageTotals::gather(&[entry_p("legacy", "", "gpt-5", &a, &[])], today());
    assert_eq!(t.by_provider[0].label, "unknown");
}

#[test]
fn gather_groups_empty_model_as_unknown() {
    let mut u = ChatUsage::default();
    u.record(UsageReport::tokens(10, 5));
    let t = UsageTotals::gather(&[entry("legacy", "", &u, &[])], today());
    assert_eq!(t.by_model.len(), 1);
    assert_eq!(t.by_model[0].label, "unknown");
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
