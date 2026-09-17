//! Headless tests for the usage dashboard: `usage_totals` folds every
//! chat's `ChatUsage` into the aggregate, the palette's "Usage Dashboard"
//! row opens the overlay, Esc and the header ✕ close it, and the usage
//! popover's "View all" row lands on the same panel. Mount pattern matches
//! `logs_tests.rs`.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::composer_testutil::open_workspace;
use crate::model::{ChatMessage, MessageKind, Role, Usage};
use crate::palette_items::{Effect, Entry};
use crate::usage::UsageReport;
use crate::workspace::Workspace;

/// Seed a chat's usage without driving a backend — the dashboard reads the
/// folded state, not the stream.
fn seed_usage(ws: &Entity<Workspace>, cx: &mut VisualTestContext, chat: usize, model: &str, reports: &[UsageReport]) {
    ws.update(cx, |this, _| {
        this.chats[chat].model = model.into();
        for &r in reports {
            this.chats[chat].usage.record(r);
        }
    });
}

#[test]
fn totals_sum_and_group_across_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(100, 40)]);
    ws.update(cx, |this, cx| {
        this.new_chat(cx);
        this.chats[this.active].model = "sonnet".into();
        this.chats[this.active].usage.record(UsageReport::tokens(50, 10));
    });
    let t = ws.read_with(cx, |ws, _| ws.usage_totals());
    assert_eq!(t.total_input, 150);
    assert_eq!(t.total_output, 50);
    assert!(!t.cost_partial, "both models are priced");
    assert!(t.total_cost > 0.);
    assert_eq!(t.by_model.len(), 2, "one row per model");
    assert_eq!(t.by_chat.len(), 2, "one row per chat with usage");
}

#[test]
fn empty_workspace_rolls_up_zeros() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let t = ws.read_with(cx, |ws, _| ws.usage_totals());
    assert_eq!(t.total_input, 0);
    assert_eq!(t.total_output, 0);
    assert_eq!(t.total_cost, 0.);
    assert!(!t.cost_partial);
    assert!(t.by_model.is_empty());
    assert!(t.by_chat.is_empty());
}

/// The palette's "Usage Dashboard" command opens the overlay; Esc and the
/// header ✕ close it.
#[test]
fn palette_command_opens_the_dashboard() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(100, 40)]);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        assert!(window.try_find("usage-dashboard-overlay").is_none(), "panel starts closed");

        let entries = crate::palette_items::build_entries(&[], "usage dashboard");
        let spec = entries
            .iter()
            .find_map(|e| match e {
                Entry::Command(spec) if spec.label == "Usage Dashboard" => Some(spec),
                _ => None,
            })
            .expect("the palette should list a Usage Dashboard command");
        let Effect::Run(run) = &spec.effect else { panic!("Usage Dashboard should run, not dispatch") };
        ws.update(cx, |this, cx| run(this, window, cx));
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).usage_dashboard_open, "the command should open the dashboard");
        assert!(window.find("usage-dashboard-overlay").visible());
        assert_eq!(window.find("usage-total-in").label().unwrap_or_default(), "Tokens in: 100");
        assert_eq!(window.find("usage-total-out").label().unwrap_or_default(), "Tokens out: 40");
        assert_eq!(window.find(("usage-model", 0usize)).label().unwrap_or_default(), "gpt-5: 100 in · 40 out · ~$0.000525");

        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).usage_dashboard_open, "esc should close the dashboard");
        assert!(window.try_find("usage-dashboard-overlay").is_none());

        // The header ✕ closes it too.
        ws.update(cx, |this, cx| this.toggle_usage_dashboard(window, cx));
        window.draw(cx).clear(cx);
        window.click("usage-dashboard-close", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).usage_dashboard_open, "header close should dismiss the panel");
    });
}

/// The usage popover's "View all" row dismisses the popover and opens the
/// dashboard.
#[test]
fn popover_view_all_opens_the_dashboard() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    seed_usage(&ws, cx, 0, "gpt-5", &[UsageReport::tokens(100, 40)]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("usage-meter", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("usage-breakdown").visible(), "meter click opens the breakdown");

        window.click("usage-view-all", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("usage-breakdown").is_none(), "the popover should dismiss");
        assert!(ws.read(cx).usage_dashboard_open, "View all should open the dashboard");
        assert!(window.find("usage-dashboard-overlay").visible());
    });
}

/// The chart reads the persisted per-message usage stamps: one bar per day
/// for the last two weeks, today highlighted, empty days as gaps.
#[test]
fn dashboard_charts_daily_usage() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let today = chrono::Local::now().date_naive();
    let two_back = today - chrono::Duration::days(2);
    let msg = |day: chrono::NaiveDate, input: u64, output: u64| ChatMessage {
        role: Role::Assistant,
        kind: MessageKind::Text("r".into()),
        rating: None,
        // Noon local — midday avoids any DST-gap ambiguity.
        at: chrono::TimeZone::from_local_datetime(&chrono::Local, &day.and_hms_opt(12, 0, 0).unwrap())
            .unwrap()
            .into(),
        usage: Some(Usage { input, output }),
        attachments: vec![],
        bookmarked: false,
        pinned: false,
        alternatives: vec![],
    };
    ws.update(cx, |this, _| {
        this.chats[0].model = "gpt-5".into();
        std::rc::Rc::make_mut(&mut this.chats[0].messages).extend([msg(today, 100, 40), msg(two_back, 50, 10)]);
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.toggle_usage_dashboard(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("usage-dashboard-chart").visible(), "the chart section renders");
        assert_eq!(window.find(format!("usage-day-{today}")).label().unwrap_or_default(), "Today: 140 tok · ~$0.000525");
        let label = |day: chrono::NaiveDate| crate::views::date_separator::day_label(day, today);
        assert_eq!(
            window.find(format!("usage-day-{two_back}")).label().unwrap_or_default(),
            format!("{}: 60 tok · ~$0.000162", label(two_back)),
        );
        // A day with no events still renders a bar slot — the gap.
        let gap = today - chrono::Duration::days(1);
        assert_eq!(window.find(format!("usage-day-{gap}")).label().unwrap_or_default(), format!("{}: 0 tok", label(gap)));
        // Bar heights are proportional: today is the peak (full height),
        // the half-usage day is shorter, the gap day has no fill.
        let h = |day: chrono::NaiveDate| window.find(format!("usage-day-fill-{day}")).bounds().size.height;
        let (peak, half, empty) = (h(today), h(two_back), h(gap));
        assert!(peak > half && half > empty, "heights should scale with usage: {peak} {half} {empty}");
        assert_eq!(empty, gpui_kit::px(0.), "an empty day renders as a gap");
    });
}
