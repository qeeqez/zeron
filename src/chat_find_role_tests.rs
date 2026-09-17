//! Tests for the find bar's role filter — the All / You / Assistant toggle
//! that narrows `find_matches`. A pure test covers `matching_messages`;
//! headless tests drive the real bar's toggle, counter, jumps and the
//! reset-on-close.
//! Declared from `chat_find.rs` via `#[path]` — `main.rs` is at the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::chat_find::matching_messages;
use crate::chat_search::find_opts::FindOpts;
use crate::chat_search::role_filter::RoleFilter;
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

fn text(role: Role, s: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind: MessageKind::Text(s.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

#[test]
fn matching_messages_narrows_by_role() {
    let messages = vec![
        text(Role::User, "hit from you"),
        text(Role::Assistant, "hit from rixl"),
        text(Role::Assistant, "hit again"),
    ];
    assert_eq!(matching_messages(&messages, "hit", RoleFilter::All, FindOpts::default()), vec![0, 1, 2], "All keeps every match");
    assert_eq!(matching_messages(&messages, "hit", RoleFilter::User, FindOpts::default()), vec![0], "You keeps only user hits");
    assert_eq!(
        matching_messages(&messages, "hit", RoleFilter::Assistant, FindOpts::default()),
        vec![1, 2],
        "Assistant keeps only its hits"
    );
    assert!(
        matching_messages(&messages, "hit", RoleFilter::User, FindOpts::default())
            .iter()
            .all(|&ix| messages[ix].role == Role::User)
    );
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-findrole-test-{}", std::process::id()));
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

/// Append a text message to the active chat and grow the scroller.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, s: &str) {
    ws.update(cx, |this, cx| {
        let chat = &mut this.chats[this.active];
        std::rc::Rc::make_mut(&mut chat.messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(s.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

/// Focus the composer and open the find bar with `query` typed.
fn open_find(ws: &Entity<Workspace>, cx: &mut VisualTestContext, query: &str) {
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("find-bar").visible(), "cmd-f should open the find bar");
    });
    // The deferred focus lands between updates; typing then fills the input.
    cx.update(|window, cx| {
        window.input(query, cx);
        window.draw(cx).clear(cx);
    });
}

#[test]
fn role_toggle_narrows_count_and_jumps() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "one hit");
    push(&ws, cx, Role::Assistant, "two hit");
    push(&ws, cx, Role::Assistant, "three hit");
    open_find(&ws, cx, "hit");
    cx.update(|window, cx| {
        assert_eq!(window.find("find-count").label(), Some("1 / 3"), "All counts every match");
        assert_eq!(window.find("find-role").label(), Some("All"));
        window.click("find-role", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-role").label(), Some("You"), "first click cycles to You");
        assert_eq!(window.find("find-count").label(), Some("1 / 1"), "only the user hit survives");
        assert_eq!(window.find(("find-hit", 0usize)).label(), Some("current find match"));
        assert!(window.try_find(("find-hit", 1usize)).is_none(), "assistant hits lose the mark");
        window.click("find-role", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-role").label(), Some("Assistant"));
        assert_eq!(window.find("find-count").label(), Some("1 / 2"), "only assistant hits survive");
        assert!(window.try_find(("find-hit", 0usize)).is_none(), "the user hit loses the mark");
        assert_eq!(window.find(("find-hit", 1usize)).label(), Some("current find match"));
        window.press("enter", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.match_ix, 1, "enter stays inside the filtered list");
        assert_eq!(window.find("find-count").label(), Some("2 / 2"));
        assert_eq!(window.find(("find-hit", 2usize)).label(), Some("current find match"));
        window.press("enter", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.match_ix, 0, "enter wraps within assistant hits, never reaching the user hit");
        window.click("find-role", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-role").label(), Some("All"), "the cycle wraps back to All");
        assert_eq!(window.find("find-count").label(), Some("1 / 3"), "All restores every match");
    });
}

#[test]
fn role_filter_resets_when_the_bar_closes() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "one hit");
    push(&ws, cx, Role::Assistant, "two hit");
    open_find(&ws, cx, "hit");
    cx.update(|window, cx| {
        window.click("find-role", cx);
        window.click("find-role", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.role, RoleFilter::Assistant);
        window.click("find-close", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).find.open);
        assert_eq!(ws.read(cx).find.role, RoleFilter::All, "closing resets the filter");
    });
    // The deferred composer focus lands between updates; cmd-f then reopens.
    cx.update(|window, cx| {
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("find-bar").visible(), "cmd-f reopens the find bar");
    });
    cx.update(|window, cx| {
        window.input("hit", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-role").label(), Some("All"), "a reopened bar starts unfiltered");
        assert_eq!(window.find("find-count").label(), Some("1 / 2"));
    });
}

#[test]
fn jump_to_message_widens_a_filter_that_hides_the_target() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "hit from you");
    push(&ws, cx, Role::Assistant, "hit from rixl");
    open_find(&ws, cx, "hit");
    cx.update(|window, cx| {
        window.click("find-role", cx);
        window.click("find-role", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.role, RoleFilter::Assistant);
        // Global search confirmed a user hit — the filter must not hide it.
        ws.update(cx, |this, cx| this.jump_to_message("hit", 0, window, cx));
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.role, RoleFilter::All, "the filter widens to reach the target");
        assert_eq!(ws.read(cx).find.match_ix, 0);
        assert_eq!(window.find(("find-hit", 0usize)).label(), Some("current find match"));
        assert_eq!(window.find("find-count").label(), Some("1 / 2"));
    });
}
