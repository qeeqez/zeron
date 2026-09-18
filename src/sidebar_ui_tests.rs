//! Headless UI tests for the shared sidebar row: the settings nav and the
//! chat list must render through the same `NavRow` component, so their rows
//! share geometry and styling. Mount pattern matches `ui_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
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

/// Settings nav rows and chat rows are the same component: identical height,
/// width and left edge inside the shared sidebar column.
#[test]
fn settings_nav_rows_share_chat_row_geometry() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let chat_id = ws.read(cx).chats[0].id;
        let chat_row = window.find(("chat-row", chat_id));
        assert!(chat_row.visible(), "chat row should render");

        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        let nav_row = window.find("settings-nav-general");
        assert!(nav_row.visible(), "settings nav row should render");

        let (chat, nav) = (chat_row.bounds(), nav_row.bounds());
        assert_eq!(nav.size.height, chat.size.height, "rows must share the same height");
        assert_eq!(nav.origin.x, chat.origin.x, "rows must share the same left padding");
        assert_eq!(nav.size.width, chat.size.width, "rows must fill the same column width");

        // Back row is the same component too, and still closes settings.
        let back = window.find("settings-back");
        assert_eq!(back.bounds().size.height, chat.size.height, "back row shares the row height");
        window.click("settings-back", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).settings_open, "back row should close settings");
        assert!(window.find(("chat-row", chat_id)).visible(), "chat list returns after settings closes");
    });
}

/// Filed chats group under a collapsible folder header; unfiled chats sit
/// under "Unfiled" below it.
#[test]
fn sidebar_groups_chats_under_folders() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (filed, unfiled) = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            let filed = this.chats[0].id;
            this.set_chat_folder(filed, "Work", cx);
            (filed, this.chats[1].id)
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let folder = window.find("group-header-Work");
        let unfiled_header = window.find("group-header-Unfiled");
        assert!(folder.visible(), "folder group header should render");
        assert!(unfiled_header.visible(), "unfiled group header should render");
        assert!(folder.bounds().origin.y < unfiled_header.bounds().origin.y, "folders lead the list");
        assert!(window.find(("chat-row", filed)).visible());
        assert!(window.find(("chat-row", unfiled)).visible());

        // Clicking the folder header folds its rows away.
        window.click("group-header-Work", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("chat-row", filed)).is_none(), "folded folder hides its chats");
        assert!(window.find(("chat-row", unfiled)).visible(), "unfiled chats stay visible");

        window.click("group-header-Work", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-row", filed)).visible(), "second click unfolds");
    });
}

/// Queue `text` on `chat_id` straight through `SendQueue` — the badge reads
/// the same map the composer renders, so the send path isn't needed.
fn enqueue(ws: &Entity<Workspace>, chat_id: u64, text: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.send_queue.enqueue(chat_id, Queued::new(text.to_string(), Vec::new()), |_| true);
            cx.notify();
        })
    });
}

/// A chat with queued sends carries a muted "+N" chip in its row; the chip
/// tracks the queue and disappears once it drains.
#[test]
fn queue_badge_tracks_queued_count() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("queue-badge", chat_id)).is_none(), "no badge without a queue");
    });

    enqueue(&ws, chat_id, "one", cx);
    enqueue(&ws, chat_id, "two", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let badge = window.find(("queue-badge", chat_id));
        assert!(badge.visible(), "badge renders for a queued chat");
        assert_eq!(badge.label(), Some("2 queued"));
    });

    // Each pop shrinks the chip; the last one removes it.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.send_queue.pop(chat_id);
            cx.notify();
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find(("queue-badge", chat_id)).label(), Some("1 queued"), "badge tracks the drain");
    });
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.send_queue.pop(chat_id);
            cx.notify();
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("queue-badge", chat_id)).is_none(), "badge hides at 0");
    });
}

/// Clicking the chip selects its chat — the queue UI lives in that chat's
/// composer — without tripping the row's own click handlers.
#[test]
fn queue_badge_click_opens_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // New chats prepend, so chats[0] is the fresh active chat and chats[1]
    // the original — queue on the background one.
    let background = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.chats[1].id
        })
    });
    enqueue(&ws, background, "later", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("queue-badge", background), cx);
        window.draw(cx).clear(cx);
        let ws = ws.read(cx);
        assert_eq!(ws.chats[ws.active].id, background, "badge click selects its chat");
        assert!(ws.renaming.is_none(), "badge click must not start a rename");
    });
}

/// A turn parked on an approval request marks its row with a warning
/// shield — the persistent "needs you" signal after the toast is gone.
/// The shield outranks the running spinner and clears once answered.
#[test]
fn approval_shield_marks_blocked_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    let (respond, _decisions) = std::sync::mpsc::channel();
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            let chat = &mut this.chats[0];
            chat.running = true;
            std::rc::Rc::make_mut(&mut chat.messages).push(crate::model::ChatMessage {
                alternatives: vec![],
                role: crate::model::Role::Assistant,
                kind: crate::model::MessageKind::Approval(crate::backend::ApprovalCard {
                    request_ix: 0,
                    kind: crate::backend::ApprovalKind::Command,
                    detail: "rm -rf ./build".into(),
                    decision: None,
                    auto_approved: false,
                    respond: Some(respond),
                }),
                rating: None,
                bookmarked: false,
                pinned: false,
                usage: None,
                attachments: vec![],
                at: std::time::SystemTime::now(),
            });
            cx.notify();
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("approval-needed", chat_id)).visible(), "blocked turn marks the row");
    });

    // Answering the request clears the marker — the plain spinner returns.
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            let msgs = std::rc::Rc::make_mut(&mut this.chats[0].messages);
            let Some(crate::model::MessageKind::Approval(a)) = msgs.last_mut().map(|m| &mut m.kind) else {
                panic!("approval card should be last")
            };
            a.respond = None;
            a.decision = Some(crate::backend::ApprovalDecision::Approve);
            cx.notify();
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("approval-needed", chat_id)).is_none(), "answered approval clears the marker");
    });
}
