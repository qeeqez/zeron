//! Tool-call grouping: a run of consecutive `Tool` messages collapses into
//! one "N tool calls" row that expands to show each call. Unit tests cover
//! `tool_group` run detection; headless tests cover the rendered transcript.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::views::cards::{ToolGroup, tool_group};
use crate::workspace::Workspace;

fn tool_msg(tool_ix: usize, name: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Tool(ToolCall {
            tool_ix,
            name: name.into(),
            detail: String::new().into(),
            output: String::new().into(),
            status: ToolStatus::Done,
            expanded: false,
        }),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn text_msg(text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

#[test]
fn consecutive_tools_form_one_group() {
    let msgs = vec![text_msg("go"), tool_msg(0, "bash"), tool_msg(1, "read"), tool_msg(2, "edit"), text_msg("done")];
    // Every member maps to the same run: head index + full length.
    for ix in 1..=3 {
        assert_eq!(tool_group(&msgs, ix), Some(ToolGroup { head: 1, len: 3 }), "ix {ix}");
    }
}

#[test]
fn non_tool_messages_break_the_run() {
    let msgs = vec![tool_msg(0, "a"), tool_msg(1, "b"), text_msg("note"), tool_msg(2, "c"), tool_msg(3, "d")];
    assert_eq!(tool_group(&msgs, 0), Some(ToolGroup { head: 0, len: 2 }));
    assert_eq!(tool_group(&msgs, 1), Some(ToolGroup { head: 0, len: 2 }));
    assert_eq!(tool_group(&msgs, 2), None, "text is never grouped");
    assert_eq!(tool_group(&msgs, 3), Some(ToolGroup { head: 3, len: 2 }));
    assert_eq!(tool_group(&msgs, 4), Some(ToolGroup { head: 3, len: 2 }));
}

#[test]
fn lone_tool_call_stays_ungrouped() {
    let msgs = vec![tool_msg(0, "a"), text_msg("note"), tool_msg(1, "b")];
    assert_eq!(tool_group(&msgs, 0), None, "single call renders its own card");
    assert_eq!(tool_group(&msgs, 2), None, "single call renders its own card");
    assert_eq!(tool_group(&msgs, 9), None, "out of range");
}

/// Mount a `Workspace` in a headless window via the shared helper.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    crate::composer_testutil::open_workspace(cx)
}

/// Push messages onto the active chat and grow the scroller to fit.
fn seed(ws: &Entity<Workspace>, msgs: Vec<ChatMessage>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        let count = msgs.len();
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).extend(msgs);
        this.scroller.update(cx, |s, cx| s.append(count, cx));
        cx.notify();
    });
}

#[test]
fn tool_run_collapses_to_group_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, vec![tool_msg(0, "bash"), tool_msg(1, "read"), tool_msg(2, "edit")], cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let group = window.find(("tool-group", 0usize));
        assert!(group.visible(), "run collapses to a summary row");
        assert_eq!(group.label(), Some("3 tool calls"));
        for ix in 0..3usize {
            assert!(window.try_find(("tool", ix)).is_none(), "call {ix} hidden while collapsed");
        }
    });
}

#[test]
fn expanding_group_reveals_each_call() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let mut with_output = tool_msg(1, "read");
    if let MessageKind::Tool(t) = &mut with_output.kind {
        t.output = "file body".into();
    }
    seed(&ws, vec![tool_msg(0, "bash"), with_output, tool_msg(2, "edit")], cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("tool-group", 0usize), cx);
        window.draw(cx).clear(cx);
        for ix in 0..3usize {
            assert!(window.find(("tool", ix)).visible(), "call {ix} shows once expanded");
        }
        // Per-call expand still works inside the group.
        window.click(("tool", 1usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("copy-tool", 1usize)).visible(), "member detail opens");
        // Collapsing the group hides the members again.
        window.click(("tool-group", 0usize), cx);
        window.draw(cx).clear(cx);
        for ix in 0..3usize {
            assert!(window.try_find(("tool", ix)).is_none(), "call {ix} hidden after collapse");
        }
    });
}

#[test]
fn text_message_breaks_the_group_visually() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, vec![tool_msg(0, "bash"), text_msg("note"), tool_msg(1, "read")], cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("tool-group", 0usize)).is_none(), "singles never group");
        assert!(window.find(("tool", 0usize)).visible(), "first call keeps its card");
        assert!(window.find(("tool", 2usize)).visible(), "call after text keeps its card");
    });
}
