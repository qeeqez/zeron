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
    // The popover's enter animation runs off the wall clock (150ms) and
    // its items only register once the surface mounts — wait it out like
    // the component's own tests do (several times the duration).
    vcx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(700));
    vcx.update(|window, cx| {
        window.draw(cx).clear(cx);
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
