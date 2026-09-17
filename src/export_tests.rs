//! Tests for "Copy transcript": the ⋯ menu item copies the whole chat as
//! markdown — `# <title>`, `**You**` / `**Assistant**` bodies verbatim,
//! cards as one-line italic summaries — and confirms with a toast. An
//! empty chat disables the item and the action leaves the clipboard
//! untouched.

use gpui_kit::base::test_support::{ElementSnapshot, snapshots};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::backend::{ApprovalCard, ApprovalDecision, ApprovalKind};
use crate::model::{ChatMessage, DiffCard, MessageKind, PlanCard, PlanStatus, PlanStep, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
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

fn msg(role: Role, kind: MessageKind) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role,
        kind,
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

fn text(role: Role, s: &str) -> ChatMessage {
    msg(role, MessageKind::Text(s.into()))
}

/// Append messages to the active chat without starting a turn.
fn seed(ws: &Entity<Workspace>, messages: Vec<ChatMessage>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).extend(messages);
        cx.notify();
    });
}

/// The ⋯ menu on the chat titlebar — opened by clicking the header button.
fn open_chat_menu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
    });
}

/// The popup-menu item snapshot with `label` — panics when it isn't offered.
fn menu_item(window: &Window, label: &str) -> ElementSnapshot {
    snapshots(window)
        .iter()
        .find(|s| s.label() == Some(label))
        .unwrap_or_else(|| panic!("menu should offer {label}"))
        .clone()
}

fn clipboard(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default())
}

/// In-app toasts mounted under the Root's notification layer.
fn toast_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| {
        let Some(Some(root)) = window.root::<Root>() else { return 0 };
        root.read(cx).notification.read(cx).notifications().len()
    })
}

#[test]
fn menu_item_copies_title_and_labeled_bodies() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "bodies");
    let title = ws.read_with(cx, |w, _| w.chats[w.active].title.to_string());
    seed(&ws, vec![text(Role::User, "how do I exit vim?"), text(Role::Assistant, "press:\n```\n:q!\n```")], cx);
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = menu_item(window, "Copy transcript");
        assert!(item.disabled() != Some(true), "a non-empty chat enables the item");
        window.within("popup-menu").click(item.path().last().unwrap().clone(), cx);
    });
    assert_eq!(
        clipboard(cx),
        format!("# {title}\n\n**You**\n\nhow do I exit vim?\n\n**Assistant**\n\npress:\n```\n:q!\n```"),
        "clipboard should hold the full markdown transcript"
    );
}

#[test]
fn cards_collapse_to_italic_summaries() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "cards");
    seed(
        &ws,
        vec![
            text(Role::User, "fix it"),
            msg(
                Role::Assistant,
                MessageKind::Tool(ToolCall {
                    tool_ix: 0,
                    name: "shell".into(),
                    detail: "cargo build".into(),
                    output: "secret build output".into(),
                    status: ToolStatus::Done,
                    expanded: false,
                }),
            ),
            msg(
                Role::Assistant,
                MessageKind::Diff(DiffCard {
                    path: "src/main.rs".into(),
                    added: 3,
                    removed: 1,
                    hunks: "@@ -1 +1 @@\n-old\n+new".into(),
                    expanded: false,
                }),
            ),
            msg(
                Role::Assistant,
                MessageKind::Plan(PlanCard {
                    plan_ix: 0,
                    steps: vec![
                        PlanStep { id: 0, label: "do thing".into(), status: PlanStatus::Done },
                        PlanStep { id: 1, label: "ship it".into(), status: PlanStatus::Pending },
                    ],
                }),
            ),
            msg(
                Role::Assistant,
                MessageKind::Approval(ApprovalCard {
                    request_ix: 0,
                    kind: ApprovalKind::Command,
                    detail: "rm -rf /tmp/x".into(),
                    decision: Some(ApprovalDecision::Approve),
                    auto_approved: false,
                    respond: None,
                }),
            ),
            text(Role::Assistant, "done"),
        ],
        cx,
    );
    cx.update(|window, cx| ws.update(cx, |this, cx| this.copy_transcript(window, cx)));
    let clip = clipboard(cx);
    for want in [
        "*ran tool: shell — cargo build*",
        "*edited `src/main.rs` (+3 -1)*",
        "*plan: 2 steps*",
        "*Run command: `rm -rf /tmp/x` — Approved*",
    ] {
        assert!(clip.contains(want), "missing {want:?} in:\n{clip}");
    }
    for gone in ["secret build output", "do thing", "@@ -1 +1 @@", "```diff"] {
        assert!(!clip.contains(gone), "card internals leaked {gone:?} into:\n{clip}");
    }
}

#[test]
fn empty_chat_disables_the_menu_item() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "empty");
    // Menu items don't expose a disabled flag to snapshots — the observable
    // is that clicking does nothing: no clipboard write, no toast.
    cx.update(|_, cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into())));
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = menu_item(window, "Copy transcript");
        window.within("popup-menu").click(item.path().last().unwrap().clone(), cx);
    });
    assert_eq!(clipboard(cx), "sentinel", "a disabled item must not run the copy");
    assert_eq!(toast_count(cx), 0, "a disabled item must not even reach the empty guard's toast");
    // The action path (app menu / palette) can't be disabled — it must
    // leave the clipboard alone and say why.
    cx.update(|window, cx| ws.update(cx, |this, cx| this.copy_transcript(window, cx)));
    assert_eq!(clipboard(cx), "sentinel", "an empty transcript must not overwrite the clipboard");
    assert_eq!(toast_count(cx), 1, "the empty copy should post a warning toast");
}

#[test]
fn copy_confirms_with_a_toast() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "toast");
    seed(&ws, vec![text(Role::User, "hi")], cx);
    cx.update(|window, cx| ws.update(cx, |this, cx| this.copy_transcript(window, cx)));
    assert_eq!(toast_count(cx), 1, "a successful copy should post a confirmation toast");
}
