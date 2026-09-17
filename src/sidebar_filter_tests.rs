//! Tests for the sidebar filter chips (`crate::sidebar_filter`): each chip's
//! predicate, the AND combination, query+chip interplay through
//! `Workspace::sidebar_visible`, and the "N of M" count. The UI test clicks
//! the Running chip and checks the list narrows — same harness as
//! `sidebar_ui_tests.rs`.

use std::rc::Rc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{Chat, ChatMessage, MessageKind, PlanCard, PlanStatus, PlanStep, Role};
use crate::sidebar_filter::{SidebarFilter, SidebarFilters};
use crate::workspace::Workspace;

fn chat(running: bool, unread: bool, plan: bool) -> Chat {
    let mut chat = Chat::new(0, "chat");
    chat.running = running;
    chat.unread = unread;
    if plan {
        Rc::make_mut(&mut chat.messages).push(ChatMessage {
            role: Role::Assistant,
            kind: MessageKind::Plan(PlanCard {
                plan_ix: 0,
                steps: vec![PlanStep { id: 0, label: "step".into(), status: PlanStatus::Pending }],
            }),
            rating: None,
            at: std::time::SystemTime::now(),
            usage: None,
            attachments: Vec::new(),
            bookmarked: false,
            pinned: false,
            alternatives: Vec::new(),
        });
    }
    chat
}

/// Each chip's predicate matches its flag and nothing else.
#[test]
fn each_filter_matches_its_flag() {
    let plain = chat(false, false, false);
    let running = chat(true, false, false);
    let unread = chat(false, true, false);
    let planned = chat(false, false, true);

    assert!(SidebarFilter::Running.matches(&running));
    assert!(!SidebarFilter::Running.matches(&unread));
    assert!(!SidebarFilter::Running.matches(&plain));

    assert!(SidebarFilter::Unread.matches(&unread));
    assert!(!SidebarFilter::Unread.matches(&running));
    assert!(!SidebarFilter::Unread.matches(&plain));

    assert!(SidebarFilter::HasPlan.matches(&planned));
    assert!(!SidebarFilter::HasPlan.matches(&running));
    assert!(!SidebarFilter::HasPlan.matches(&plain));
}

/// Chips AND together: Running+Unread passes only a chat that is both.
#[test]
fn active_filters_and_together() {
    let mut filters = SidebarFilters::default();
    filters.toggle(SidebarFilter::Running);
    filters.toggle(SidebarFilter::Unread);

    let both = chat(true, true, false);
    assert!(filters.matches(&both));
    assert!(!filters.matches(&chat(true, false, false)), "running alone fails the Unread chip");
    assert!(!filters.matches(&chat(false, true, false)), "unread alone fails the Running chip");
    assert!(!filters.matches(&chat(false, false, false)));

    // An empty set filters nothing.
    let mut filters = SidebarFilters::default();
    assert!(!filters.any());
    assert!(filters.matches(&chat(false, false, false)));

    // Toggling the same chip twice turns it back off.
    filters.toggle(SidebarFilter::Running);
    assert!(filters.is_active(SidebarFilter::Running));
    filters.toggle(SidebarFilter::Running);
    assert!(!filters.is_active(SidebarFilter::Running));
    assert!(!filters.any());
}

/// The count label appears only while chips are on.
#[test]
fn count_text_only_when_active() {
    let mut filters = SidebarFilters::default();
    assert_eq!(filters.count_text(3, 12), None);
    filters.toggle(SidebarFilter::Running);
    assert_eq!(filters.count_text(3, 12).as_deref(), Some("3 of 12"));
    assert_eq!(filters.count_text(0, 12).as_deref(), Some("0 of 12"));
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-sidebar-filter-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

/// Chips combine with the title query through `sidebar_visible`: a chat must
/// pass every active chip AND match the query text.
#[test]
fn query_and_filters_intersect() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[0].title = "alpha".into();
            this.chats[0].running = true;
            this.chats[1].title = "beta".into();
            this.chats[1].running = true;
            this.chats[1].unread = true;

            // Query alone narrows by title; an empty chip set filters
            // nothing. Order is newest-first (beta was created second).
            assert_eq!(this.sidebar_visible("alp"), vec![0]);
            assert_eq!(this.sidebar_visible(""), vec![1, 0]);

            // Chip alone narrows by flag.
            this.sidebar_filters.toggle(SidebarFilter::Unread);
            assert_eq!(this.sidebar_visible(""), vec![1], "Unread chip leaves only the unread chat");

            // Query AND chip: alpha is running but read, so it drops out.
            assert_eq!(this.sidebar_visible("alp"), Vec::<usize>::new());
            assert_eq!(this.sidebar_visible("bet"), vec![1]);

            // Running+Unread: only the chat that is both survives.
            this.sidebar_filters.toggle(SidebarFilter::Running);
            assert_eq!(this.sidebar_visible(""), vec![1]);

            // Dropping Unread leaves Running alone — both chats qualify.
            this.sidebar_filters.toggle(SidebarFilter::Unread);
            assert_eq!(this.sidebar_visible(""), vec![1, 0]);
        });
    });
}

/// Clicking the Running chip narrows the rendered list to running chats,
/// shows the "N of M" count, and a second click restores the list.
#[test]
fn running_chip_filters_the_list() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (running_id, idle_id) = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[0].running = true;
            (this.chats[0].id, this.chats[1].id)
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-row", running_id)).visible());
        assert!(window.find(("chat-row", idle_id)).visible());
        assert!(window.try_find("sidebar-filter-count").is_none(), "no count while chips are off");

        window.click("sidebar-filter-running", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-row", running_id)).visible(), "the running chat stays listed");
        assert!(window.try_find(("chat-row", idle_id)).is_none(), "the idle chat is filtered out");
        assert!(window.find("sidebar-filter-count").visible(), "the count appears while a chip is on");
        assert!(ws.read(cx).sidebar_filters.is_active(SidebarFilter::Running), "the click toggled the chip on");

        window.click("sidebar-filter-running", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-row", idle_id)).visible(), "toggling off restores the list");
    });
}

/// A filter that matches nothing renders the muted "No chats match" row.
#[test]
fn no_match_shows_empty_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("sidebar-no-match").is_none(), "no placeholder on the unfiltered list");

        window.click("sidebar-filter-unread", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("sidebar-no-match").visible(), "empty result shows the placeholder row");

        window.click("sidebar-filter-unread", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("sidebar-no-match").is_none(), "clearing the chip removes the placeholder");
    });
    let _ = ws;
}
