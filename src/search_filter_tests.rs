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

/// One searchable doc: `texts` become user messages stamped `ages[ix]`
/// seconds ago so the date filters have something to bite on.
fn doc(provider: &str, model: &str, texts: &[&str], ages: &[u64]) -> SearchDoc {
    assert_eq!(texts.len(), ages.len());
    let now = SystemTime::now();
    let messages = texts
        .iter()
        .zip(ages)
        .map(|(t, age)| ChatMessage {
            role: Role::User,
            kind: MessageKind::Text((*t).into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at: now - Duration::from_secs(*age),
        })
        .collect();
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

/// Append a text message to the active chat — works inside `cx.update`
/// where only `&mut App` is available.
fn push_to(this: &mut Workspace, s: &str) {
    std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
        role: Role::User,
        kind: MessageKind::Text(s.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: SystemTime::now(),
    });
}

/// Click the item `label` inside the popup-menu that `chip` opened — the
/// sibling chips' menus can stay mounted, so the item's leaf id is only
/// unique inside its own popover scope. Takes the test context (not a
/// window borrow) so the dismiss animation can park before the next chip
/// click — a still-closing popover would toggle shut instead of opening.
fn click_menu_item(vcx: &mut VisualTestContext, chip: &str, label: &str) {
    vcx.update(|window, cx| {
        let popover = format!("popover:dropdown-menu:Name(\"{chip}\")");
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some(label) && s.path().iter().any(|id| *id == popover.clone().into()))
            .unwrap_or_else(|| {
                let labels: Vec<_> = snapshots(window)
                    .iter()
                    .filter(|s| s.path().iter().any(|id| *id == popover.clone().into()))
                    .map(|s| s.label().unwrap_or("?").to_string())
                    .collect();
                panic!("menu should offer {label} — offers: {labels:?}")
            })
            .clone();
        let leaf = item.path().last().unwrap().clone();
        window.within(gpui_kit::ElementId::Name(popover.into())).click(leaf, cx);
    });
    vcx.run_until_parked();
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
    let from = SearchFilters { date_from: Some(now - Duration::from_secs(5 * day)), ..Default::default() };
    assert_eq!(search(&docs, "needle", &from).len(), 2, "date_from drops the oldest hit");
    let to = SearchFilters { date_to: Some(now - Duration::from_secs(5 * day)), ..Default::default() };
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
    let docs = vec![doc("prov-a", "model-a", &["needle one"], &[3_600]), doc("prov-b", "model-b", &["needle two"], &[10 * 86_400])];
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
        window.draw(cx).clear(cx);
        window.press("cmd-shift-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("search-filters").visible(), "the dialog shows the filter row");
        ws.update(cx, |this, cx| {
            this.global_search.update(cx, |state, cx| state.set_query("needle", window, cx));
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "unfiltered: both chats match");
        // The Provider chip narrows to prov-b's chat — unconfigured ids
        // render their raw id as the menu label.
        window.click("search-filter-provider", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-provider", "prov-b");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "provider filter narrows to one chat");
        // The Model chip ANDs on top of the provider pick.
        window.click("search-filter-model", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-model", "model-b");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1);
        // Clearing each chip restores the wider result set.
        window.click("search-filter-provider", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-provider", "Any provider");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "the model filter still applies");
        window.click("search-filter-model", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-model", "Any model");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "clearing both restores every hit");
    });
}
