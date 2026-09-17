//! Tests for `chat_find` — the Cmd-F transcript find bar. Pure tests cover
//! match computation and index stepping; headless tests drive the real bar
//! (open, type, cycle, close) in a mounted workspace.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::chat_find::{matching_messages, step_ix};
use crate::chat_search::find_opts::FindOpts;
use crate::chat_search::role_filter::RoleFilter;
use crate::model::{ChatMessage, MessageKind, PlanCard, PlanStatus, PlanStep, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

fn msg(kind: MessageKind) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind,
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn text(s: &str) -> ChatMessage {
    msg(MessageKind::Text(s.into()))
}

#[test]
fn matching_messages_covers_message_kinds() {
    let messages = vec![
        text("hello world"),
        msg(MessageKind::Plan(PlanCard {
            plan_ix: 0,
            steps: vec![
                PlanStep { id: 0, label: "scan repo".into(), status: PlanStatus::Done },
                PlanStep { id: 1, label: "fix bug".into(), status: PlanStatus::Pending },
            ],
        })),
        msg(MessageKind::Tool(ToolCall {
            tool_ix: 0,
            name: "shell".into(),
            detail: "cargo test".into(),
            output: "3 passed".into(),
            status: ToolStatus::Done,
            expanded: false,
        })),
        text("unrelated"),
    ];
    assert_eq!(matching_messages(&messages, "hello", RoleFilter::All, FindOpts::default()), vec![0]);
    assert_eq!(matching_messages(&messages, "fix", RoleFilter::All, FindOpts::default()), vec![1], "plan step labels match");
    assert_eq!(matching_messages(&messages, "passed", RoleFilter::All, FindOpts::default()), vec![2], "tool output matches");
    assert_eq!(matching_messages(&messages, "shell", RoleFilter::All, FindOpts::default()), vec![2], "tool name matches");
    assert_eq!(matching_messages(&messages, "o", RoleFilter::All, FindOpts::default()), vec![0, 1, 2], "transcript order preserved");
}

#[test]
fn matching_messages_is_case_insensitive() {
    let messages = vec![text("Hello World")];
    assert_eq!(matching_messages(&messages, "HELLO", RoleFilter::All, FindOpts::default()), vec![0]);
    assert_eq!(matching_messages(&messages, "world", RoleFilter::All, FindOpts::default()), vec![0]);
    assert!(matching_messages(&messages, "bye", RoleFilter::All, FindOpts::default()).is_empty());
}

#[test]
fn matching_messages_empty_query_matches_nothing() {
    let messages = vec![text("anything")];
    assert!(matching_messages(&messages, "", RoleFilter::All, FindOpts::default()).is_empty());
}

#[test]
fn step_ix_wraps_both_directions() {
    assert_eq!(step_ix(0, false, 3), 1);
    assert_eq!(step_ix(2, false, 3), 0, "next wraps past the last match");
    assert_eq!(step_ix(0, true, 3), 2, "prev wraps before the first match");
    assert_eq!(step_ix(1, true, 3), 0);
    assert_eq!(step_ix(5, false, 3), 0, "stale index clamps to last, then wraps");
    assert_eq!(step_ix(0, false, 0), 0, "no matches is a no-op");
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-chatfind-test-{}", std::process::id()));
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

#[test]
fn cmd_f_opens_find_bar_and_typing_counts_matches() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "alpha beta");
    push(&ws, cx, Role::Assistant, "beta gamma");
    push(&ws, cx, Role::Assistant, "no hit here");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("find-bar").visible(), "cmd-f should open the find bar");
        assert!(ws.read(cx).find.open);
    });
    // The deferred focus lands between updates; typing then fills the input.
    cx.update(|window, cx| {
        window.input("beta", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("1 / 2"), "two messages contain beta");
        assert_eq!(window.find(("find-hit", 0usize)).label(), Some("current find match"));
        assert_eq!(window.find(("find-hit", 1usize)).label(), Some("find match"));
        assert!(window.try_find(("find-hit", 2usize)).is_none(), "non-matching message is not marked");
    });
}

#[test]
fn enter_cycles_matches_and_shift_enter_goes_back() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "one hit");
    push(&ws, cx, Role::Assistant, "two hit");
    push(&ws, cx, Role::Assistant, "three hit");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.press("cmd-f", cx);
    });
    cx.update(|window, cx| {
        window.input("hit", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("find-count").label(), Some("1 / 3"));
        window.press("enter", cx);
    });
    // PressEnter is emitted as an effect — it lands on the next flush.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.match_ix, 1);
        assert_eq!(window.find("find-count").label(), Some("2 / 3"));
        assert_eq!(window.find(("find-hit", 1usize)).label(), Some("current find match"));
        window.press("enter", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.match_ix, 0, "next past the last match wraps");
        assert_eq!(window.find("find-count").label(), Some("1 / 3"));
        window.press("shift-enter", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).find.match_ix, 2, "shift-enter before the first match wraps back");
        assert_eq!(window.find(("find-hit", 2usize)).label(), Some("current find match"));
    });
}

#[test]
fn escape_closes_find_bar() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "find me");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("find-bar").visible());
    });
    cx.update(|window, cx| {
        window.input("find", cx);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("find-bar").is_none(), "esc should close the find bar");
        assert!(!ws.read(cx).find.open);
        assert!(window.try_find(("find-hit", 0usize)).is_none(), "closing clears the highlight");
    });
}

#[test]
fn close_button_closes_find_bar() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "find me");
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.press("cmd-f", cx);
        window.draw(cx).clear(cx);
        window.click("find-close", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("find-bar").is_none(), "the ✕ button should close the bar");
        assert!(!ws.read(cx).find.open);
    });
}
