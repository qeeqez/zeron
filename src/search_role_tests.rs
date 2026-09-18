//! Tests for the global-search Role chip — the All / You / Assistant
//! filter that narrows `search` hits by message role. A pure test drives
//! `search` with `SearchFilters.role` directly; the headless test mounts a
//! workspace, opens the real dialog and clicks the chip's menu.
//! Declared from `global_search.rs` via `#[path]` — `main.rs` is at the
//! SLOC cap.

use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::chat_search::role_filter::RoleFilter;
use crate::global_search::{SearchDoc, SearchFilters, search};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// One searchable doc whose `texts[ix]` is a message of `roles[ix]`.
fn roled_doc(texts: &[&str], roles: &[Role]) -> SearchDoc {
    assert_eq!(texts.len(), roles.len());
    let messages = texts
        .iter()
        .zip(roles)
        .map(|(t, role)| ChatMessage {
            role: *role,
            kind: MessageKind::Text((*t).into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            alternatives: vec![],
            at: SystemTime::now(),
        })
        .collect();
    SearchDoc {
        chat_id: None,
        file_ix: 0,
        title: "Chat".into(),
        provider: "prov".into(),
        model: "model".into(),
        messages: Rc::new(messages),
    }
}

#[test]
fn role_filter_keeps_only_that_role() {
    let docs = vec![roled_doc(
        &["needle from you", "needle from rixl", "needle again"],
        &[Role::User, Role::Assistant, Role::Assistant],
    )];
    let all = search(&docs, "needle", &SearchFilters::default());
    assert_eq!(all.len(), 3, "an all-default filter keeps every role");
    let you = SearchFilters { role: RoleFilter::User, ..Default::default() };
    let hits = search(&docs, "needle", &you);
    assert_eq!(hits.len(), 1, "You keeps only the user hit");
    assert_eq!(hits[0].msg_ix, 0);
    let assistant = SearchFilters { role: RoleFilter::Assistant, ..Default::default() };
    let hits = search(&docs, "needle", &assistant);
    assert_eq!(hits.len(), 2, "Assistant keeps only its hits");
    assert_eq!(hits[0].msg_ix, 2, "hits stay newest-first");
    assert_eq!(hits[1].msg_ix, 1);
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-search-role-test-{}", std::process::id()));
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

/// Append a text message of `role` to the active chat — works inside
/// `cx.update` where only `&mut App` is available.
fn push_to(this: &mut Workspace, role: Role, s: &str) {
    std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
        role,
        kind: MessageKind::Text(s.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        alternatives: vec![],
        at: SystemTime::now(),
    });
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
            window.draw(cx).clear(cx);
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
fn role_chip_narrows_live_results() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            push_to(this, Role::User, "needle from you");
            push_to(this, Role::Assistant, "needle from rixl");
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.press("cmd-shift-f", cx);
    });
    // Same wall-clock enter animation as the filter test — settle painted
    // bounds so the chip click can't land on the dismissable backdrop.
    crate::composer_testutil::settle_dialog(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("search-filter-role").visible(), "the dialog shows the Role chip");
        ws.update(cx, |this, cx| {
            this.global_search.update(cx, |state, cx| state.set_query("needle", window, cx));
        });
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "unfiltered: both roles match");
        window.click("search-filter-role", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-role", "Assistant");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "Assistant keeps only its hit");
        window.click("search-filter-role", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-role", "You");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 1, "You keeps only the user hit");
        window.click("search-filter-role", cx);
        window.draw(cx).clear(cx);
    });
    click_menu_item(cx, "search-filter-role", "Any role");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).global_search.read(cx).matched_count(), 2, "Any role restores every hit");
    });
}
