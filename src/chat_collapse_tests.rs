//! Tests for `chat_collapse` — long messages clipped behind a "Show more"
//! bar. Pure tests cover the line estimate and classification; headless
//! tests drive the mounted workspace (toggle state, the rendered bar, and
//! the search-jump auto-expand).

// Explicit imports, not `gpui_kit::*` — the glob re-exports GPUI's `test`
// attribute macro, which would shadow Rust's `#[test]` here.
use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::chat_collapse::{COLLAPSE_LINES, Collapse, collapse_state, collapsible, rendered_lines};
use crate::workspace::Workspace;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

fn msg(kind: MessageKind) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind,
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn text(s: &str) -> ChatMessage {
    msg(MessageKind::Text(s.into()))
}

/// A message past the collapse threshold.
fn long_text() -> ChatMessage {
    text(&(0..COLLAPSE_LINES + 5).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n"))
}

#[test]
fn rendered_lines_counts_source_lines() {
    assert_eq!(rendered_lines("one\ntwo\nthree", true), 3);
    assert_eq!(rendered_lines("", true), 0);
}

#[test]
fn rendered_lines_estimates_wraps_only_when_wrapping() {
    let wide = "x".repeat(250);
    assert_eq!(rendered_lines(&wide, true), 3, "250 chars wraps to 3 rows at 100 cols");
    assert_eq!(rendered_lines(&wide, false), 1, "nowrap renders one row however wide");
}

#[test]
fn collapsible_gates_on_threshold_kind_and_streaming() {
    let short = text("hello");
    let long = long_text();
    assert!(!collapsible(&short, true, false), "short text never collapses");
    assert!(collapsible(&long, true, false), "long text collapses");
    assert!(!collapsible(&long, true, true), "the streaming tail stays expanded");
    let card = msg(MessageKind::Plan(crate::model::PlanCard { plan_ix: 0, steps: vec![] }));
    assert!(!collapsible(&card, true, false), "non-text kinds never collapse");
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-collapse-test-{}", std::process::id()));
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

/// Append a message to the active chat and grow the scroller.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, m: ChatMessage) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(m);
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// Whether message `ix` renders collapsed right now.
fn collapsed(ws: &Entity<Workspace>, cx: &mut VisualTestContext, ix: usize) -> bool {
    ws.read_with(cx, |this, _| {
        let msg = &this.chats[this.active].messages[ix];
        matches!(collapse_state(this, ix, msg, false), Collapse::Collapsed)
    })
}

#[test]
fn toggle_expands_and_recollapses() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, long_text());
    assert!(collapsed(&ws, cx, 0), "a long message starts collapsed");

    let at = ws.read_with(cx, |this, _| this.chats[this.active].messages[0].at);
    ws.update(cx, |this, cx| this.toggle_msg_collapse(0, at, cx));
    assert!(!collapsed(&ws, cx, 0), "Show more expands in place");

    ws.update(cx, |this, cx| this.toggle_msg_collapse(0, at, cx));
    assert!(collapsed(&ws, cx, 0), "Show less re-collapses");
}

#[test]
fn expand_msg_ignores_short_and_streaming() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, text("short"));
    ws.update(cx, |this, cx| this.expand_msg(0, cx));
    assert!(ws.read_with(cx, |this, _| this.chats[this.active].expanded_msgs.is_empty()), "short messages never enter the set");

    ws.update(cx, |this, _| this.chats[this.active].running = true);
    push(&ws, cx, long_text());
    ws.update(cx, |this, cx| this.expand_msg(1, cx));
    assert!(ws.read_with(cx, |this, _| this.chats[this.active].expanded_msgs.is_empty()), "the streaming tail is not collapsible");
}

#[test]
fn scroll_to_message_expands_collapsed_hit() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, long_text());
    push(&ws, cx, text("tail"));
    assert!(collapsed(&ws, cx, 0));

    // The shared jump used by find, global search and message navigation.
    ws.update(cx, |this, cx| this.scroll_to_message(0, cx));
    assert!(!collapsed(&ws, cx, 0), "navigating to a collapsed message expands it");
}

#[test]
fn chat_search_jump_expands_collapsed_match() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, text("needle one"));
    push(&ws, cx, {
        let mut m = long_text();
        if let MessageKind::Text(t) = &mut m.kind {
            *t = format!("{t}\nneedle buried deep").into();
        }
        m
    });
    assert!(collapsed(&ws, cx, 1));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.chat_search_open = true;
            this.chat_search.update(cx, |s, cx| s.set_value("needle", window, cx));
            // Enter advances to the next match — the first jump lands on
            // the second match (the collapsed long message).
            this.jump_to_match(false, cx);
        });
    });
    assert_eq!(ws.read_with(cx, |this, _| this.search_match_ix), 1);
    assert!(!collapsed(&ws, cx, 1), "jumping to a collapsed match expands it");
    assert_eq!(ws.read_with(cx, |this, _| this.chats[this.active].expanded_msgs.len()), 1, "only the jumped-to match expanded");
}

#[gpui_kit::test]
fn long_message_renders_show_more_bar(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    push(&ws, cx, text("short"));
    push(&ws, cx, long_text());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("msg-collapse", 1usize)).visible(), "a long message shows the expand bar");
        assert!(window.try_find(("msg-collapse", 0usize)).is_none(), "a short message has no bar");
        window.click(("msg-collapse", 1usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("msg-collapse", 1usize)).visible(), "the bar stays as Show less");
    });
    assert!(!collapsed(&ws, cx, 1), "clicking Show more expanded the message");
}

#[gpui_kit::test]
fn streaming_tail_never_collapses(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    ws.update(cx, |this, _| this.chats[this.active].running = true);
    push(&ws, cx, long_text());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("msg-collapse", 0usize)).is_none(), "no collapse bar mid-stream");
    });
    ws.update(cx, |this, cx| {
        this.chats[this.active].running = false;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("msg-collapse", 0usize)).visible(), "the finished reply collapses");
    });
}
//
