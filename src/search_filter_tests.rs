//! Tests for the global-search filter row — the Date/Model/Provider chips
//! that narrow `search` results. Pure tests drive `search` with
//! `SearchFilters` values directly; the headless test mounts a workspace,
//! opens the real dialog and clicks the chips' menus.
//! Declared from `global_search.rs` via `#[path]` — `main.rs` is at the
//! SLOC cap.

use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::global_search::{DateRange, SearchDoc, SearchFilters, search};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// A user text message stamped `age_secs` seconds ago.
fn msg_at(s: &str, age_secs: u64) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        kind: MessageKind::Text(s.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        alternatives: vec![],
        at: SystemTime::now() - Duration::from_secs(age_secs),
    }
}

/// One searchable doc: `texts` become user messages stamped `ages[ix]`
/// seconds ago so the date filters have something to bite on.
fn doc(provider: &str, model: &str, texts: &[&str], ages: &[u64]) -> SearchDoc {
    assert_eq!(texts.len(), ages.len());
    let messages = texts.iter().zip(ages).map(|(t, age)| msg_at(t, *age)).collect();
    SearchDoc {
        chat_id: None,
        file_ix: 0,
        title: "Chat".into(),
        provider: provider.into(),
        model: model.into(),
        messages: Rc::new(messages),
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-search-filter-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// Repaint so element snapshots reflect the latest state.
fn redraw(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.draw(cx).clear(cx);
}

/// Append a text message to the active chat — works inside `cx.update`
/// where only `&mut App` is available.
fn push_to(this: &mut Workspace, s: &str) {
    std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(msg_at(s, 0));
}

/// Click the item `label` inside the popup-menu that `chip` opened — the
/// sibling chips' menus can stay mounted, so the item's leaf id is only
/// unique inside its own popover scope. Takes the test context (not a
/// window borrow) so the dismiss animation can park before the next chip
/// click — a still-closing popover would toggle shut instead of opening.
fn click_menu_item(vcx: &mut VisualTestContext, chip: &str, label: &str) {
    // The popover element wraps its always-mounted trigger, so the menu is
    // open iff the item is in the snapshot — PopupMenu builds items the
    // frame it opens and there is no exit animation. Item absent for a
    // while means the chip click was swallowed, so re-click it. The item
    // must stay visible through the 150ms enter animation before being
    // clicked: a mid-slide surface puts the pointer where the item isn't
    // yet, and a miss changes nothing (overlay_closable is off), so the
    // click retries until the menu closes — item activation is the only
    // thing that dismisses it. The chip itself is excluded from the match:
    // it shows the picked label and sits under the popover as the trigger,
    // so it would pass for a menu item once a selection lands.
    let popover = format!("popover:dropdown-menu:Name(\"{chip}\")");
    let chip_id: gpui_kit::ElementId = chip.to_string().into();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let (mut absent_since, mut seen_since, mut clicked_item) = (std::time::Instant::now(), None, false);
    loop {
        vcx.run_until_parked();
        let (present, clicked) = vcx.update(|window, cx| {
            redraw(window, cx);
            let leaf = snapshots(window)
                .iter()
                .find(|s| {
                    s.label() == Some(label) && s.path().iter().any(|id| *id == popover.clone().into()) && s.path().last() != Some(&chip_id)
                })
                .and_then(|s| s.path().last().cloned());
            let Some(leaf) = leaf else { return (false, false) };
            if seen_since.is_none_or(|t: std::time::Instant| t.elapsed() < std::time::Duration::from_millis(250)) {
                return (true, false);
            }
            window.within(gpui_kit::ElementId::Name(popover.clone().into())).click(leaf, cx);
            (true, true)
        });
        if !present {
            if clicked_item {
                return;
            }
            seen_since = None;
            if absent_since.elapsed() > std::time::Duration::from_secs(1) {
                absent_since = std::time::Instant::now();
                vcx.update(|window, cx| window.click(chip.to_string(), cx));
            }
        } else {
            absent_since = std::time::Instant::now();
            if clicked {
                clicked_item = true;
            } else {
                seen_since.get_or_insert_with(std::time::Instant::now);
            }
        }
        assert!(std::time::Instant::now() < deadline, "menu should offer {label}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn empty_filters_keep_every_match() {
    let docs = vec![doc("prov-a", "model-a", &["needle one"], &[60]), doc("prov-b", "model-b", &["needle two"], &[120])];
    let hits = search(&docs, "needle", &SearchFilters::default());
    assert_eq!(hits.len(), 2, "an all-default filter changes nothing");
}

#[test]
fn model_filter_keeps_only_that_model() {
    let docs = vec![doc("prov-a", "model-a", &["needle one"], &[60]), doc("prov-b", "model-b", &["needle two"], &[120])];
    let filters = SearchFilters { model: Some("model-a".into()), ..Default::default() };
    let hits = search(&docs, "needle", &filters);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].model, "model-a");
    assert_eq!(hits[0].provider, "prov-a", "the hit carries its chat's stamps");
}

#[test]
fn provider_filter_keeps_only_that_provider() {
    let docs = vec![doc("prov-a", "model-a", &["needle one"], &[60]), doc("prov-b", "model-b", &["needle two"], &[120])];
    let filters = SearchFilters { provider: Some("prov-b".into()), ..Default::default() };
    let hits = search(&docs, "needle", &filters);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].provider, "prov-b");
    assert_eq!(hits[0].model, "model-b");
}

#[test]
fn date_filters_bound_the_window() {
    let day = 86_400;
    let docs = vec![doc("prov-a", "model-a", &["needle fresh", "needle mid", "needle old"], &[3_600, 3 * day, 10 * day])];
    let now = SystemTime::now();
    let from = SearchFilters {
        date_from: Some(now - Duration::from_secs(5 * day)),
        ..Default::default()
    };
    assert_eq!(search(&docs, "needle", &from).len(), 2, "date_from drops the oldest hit");
    let to = SearchFilters {
        date_to: Some(now - Duration::from_secs(5 * day)),
        ..Default::default()
    };
    let hits = search(&docs, "needle", &to);
    assert_eq!(hits.len(), 1, "date_to keeps only the oldest hit");
    assert_eq!(hits[0].msg_ix, 2);
    let both = SearchFilters {
        date_from: Some(now - Duration::from_secs(5 * day)),
        date_to: Some(now - Duration::from_secs(2 * day)),
        ..Default::default()
    };
    let hits = search(&docs, "needle", &both);
    assert_eq!(hits.len(), 1, "the range keeps only the middle hit");
    assert_eq!(hits[0].msg_ix, 1);
}

#[test]
fn date_preset_filters_rolling_window() {
    let docs = vec![doc("prov-a", "model-a", &["needle fresh", "needle old"], &[3_600, 3 * 86_400])];
    let today = SearchFilters { date: DateRange::Day, ..Default::default() };
    assert_eq!(search(&docs, "needle", &today).len(), 1, "Today keeps the last 24h");
    let week = SearchFilters { date: DateRange::Week, ..Default::default() };
    assert_eq!(search(&docs, "needle", &week).len(), 2, "This week reaches back 7 days");
}

#[test]
fn filters_combine_with_and() {
    let docs = vec![
        doc("prov-a", "model-a", &["needle a"], &[3_600]),
        doc("prov-a", "model-b", &["needle b"], &[3_600]),
        doc("prov-b", "model-a", &["needle c"], &[10 * 86_400]),
    ];
    let filters = SearchFilters {
        provider: Some("prov-a".into()),
        model: Some("model-a".into()),
        date_from: Some(SystemTime::now() - Duration::from_secs(86_400)),
        ..Default::default()
    };
    let hits = search(&docs, "needle", &filters);
    assert_eq!(hits.len(), 1, "every clause must hold — only the first doc qualifies");
    assert_eq!(hits[0].provider, "prov-a");
    assert_eq!(hits[0].model, "model-a");
}

#[test]
fn clearing_filters_restores_all_hits() {
    let docs = vec![
        doc("prov-a", "model-a", &["needle one"], &[3_600]),
        doc("prov-b", "model-b", &["needle two"], &[10 * 86_400]),
    ];
    let filters = SearchFilters { provider: Some("prov-a".into()), ..Default::default() };
    assert_eq!(search(&docs, "needle", &filters).len(), 1);
    let cleared = SearchFilters { provider: None, ..filters };
    assert_eq!(search(&docs, "needle", &cleared).len(), 2, "clearing the filter restores every hit");
}

#[test]
fn filter_chips_narrow_live_results() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.chats[this.active].provider = "prov-a".into();
            this.chats[this.active].model = "model-a".into();
            push_to(this, "needle in first chat");
            this.new_chat(cx);
            this.chats[this.active].provider = "prov-b".into();
            this.chats[this.active].model = "model-b".into();
            push_to(this, "needle in second chat");
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.bind_keys(crate::workspace_keys());
        redraw(window, cx);
        window.press("cmd-shift-f", cx);
    });
    // The dialog's enter animation runs off the wall clock — a chip click
    // dispatched mid-slide lands on the dismissable backdrop and pops the
    // dialog. Wait for painted bounds to stop moving first.
    crate::composer_testutil::settle_dialog(cx);
    cx.update(|window, cx| {
        redraw(window, cx);
        assert!(window.find("search-filters").visible(), "the dialog shows the filter row");
        ws.update(cx, |this, cx| {
            this.global_search.update(cx, |state, cx| state.set_query("needle", window, cx));
        });
        redraw(window, cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "unfiltered: both chats match");
        // The Provider chip narrows to prov-b's chat — unconfigured ids
        // render their raw id as the menu label.
        window.click("search-filter-provider", cx);
        redraw(window, cx);
    });
    click_menu_item(cx, "search-filter-provider", "prov-b");
    cx.update(|window, cx| {
        redraw(window, cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "provider filter narrows to one chat");
        // The Model chip ANDs on top of the provider pick.
        window.click("search-filter-model", cx);
        redraw(window, cx);
    });
    click_menu_item(cx, "search-filter-model", "model-b");
    cx.update(|window, cx| {
        redraw(window, cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1);
        // Clearing each chip restores the wider result set.
        window.click("search-filter-provider", cx);
        redraw(window, cx);
    });
    click_menu_item(cx, "search-filter-provider", "Any provider");
    cx.update(|window, cx| {
        redraw(window, cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "the model filter still applies");
        window.click("search-filter-model", cx);
        redraw(window, cx);
    });
    click_menu_item(cx, "search-filter-model", "Any model");
    cx.update(|window, cx| {
        redraw(window, cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "clearing both restores every hit");
    });
}
