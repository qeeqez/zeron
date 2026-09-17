//! Headless tests for chat drag-reorder: `drop_chat_on_row` renumbers the
//! target group, the new order drives `sidebar_order`, and `Chat::order`
//! round-trips through `save_chats`/`load_chats` so a custom order survives
//! restarts. Mount pattern matches `chat_ops_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-reorder-test-{}", std::process::id()));
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

/// Three chats with distinct `created_at`s — the workspace's initial chat
/// plus two more. Returns ids in sidebar order (newest first).
fn three_chats(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.new_chat(cx);
            let n = this.chats.len() as u64;
            for (i, chat) in this.chats.iter_mut().enumerate() {
                chat.created_at = std::time::SystemTime::now() - std::time::Duration::from_secs(n - i as u64);
            }
            this.sidebar_order("").iter().map(|ix| this.chats[*ix].id).collect()
        })
    })
}

/// Sidebar order as chat ids.
fn sidebar_ids(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| ws.read(cx).sidebar_order("").iter().map(|ix| ws.read(cx).chats[*ix].id).collect())
}

/// Dropping the newest chat below the oldest reorders the bucket and the
/// sidebar reflects it.
#[test]
fn drop_below_reorders_within_bucket() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = three_chats(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.chat_drop = Some(crate::workspace::ChatDrop { row: ids[2], above: false });
            this.drop_chat_on_row(ids[0], ids[2], cx);
        })
    });
    assert_eq!(sidebar_ids(&ws, cx), vec![ids[1], ids[2], ids[0]], "dragged chat lands below the target");
}

/// Dropping the oldest chat above the newest puts it on top.
#[test]
fn drop_above_reorders_to_top() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = three_chats(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.chat_drop = Some(crate::workspace::ChatDrop { row: ids[0], above: true });
            this.drop_chat_on_row(ids[2], ids[0], cx);
        })
    });
    assert_eq!(sidebar_ids(&ws, cx), vec![ids[2], ids[0], ids[1]], "dragged chat lands above the target");
}

/// A cross-bucket drop in the flat list is rejected — bucket membership is
/// derived from `created_at`, so there's nothing to reorder into.
#[test]
fn cross_bucket_drop_is_rejected() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = three_chats(&ws, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            // Age the newest chat out of "Today" into "Older".
            let ix = this.chat_index(ids[0]).unwrap();
            this.chats[ix].created_at = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400);
        })
    });
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            assert!(!this.can_drop_chat_on(ids[1], ids[0]), "cross-bucket drop must be rejected");
            this.chat_drop = Some(crate::workspace::ChatDrop { row: ids[0], above: false });
            this.drop_chat_on_row(ids[1], ids[0], cx);
        })
    });
    assert_eq!(sidebar_ids(&ws, cx), vec![ids[1], ids[2], ids[0]], "rejected drop leaves the order alone");
}

/// A reorder writes `Chat::order`, which round-trips through the chats dir —
/// the manual order survives a relaunch.
#[test]
fn reorder_persists_through_save_load() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = three_chats(&ws, cx);
    let dir = cx.update(|_, cx| ws.read(cx).project.chats_dir());
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            for (i, id) in ids.iter().enumerate() {
                let ix = this.chat_index(*id).unwrap();
                this.chats[ix].title = format!("chat-{i}").into();
            }
            this.chat_drop = Some(crate::workspace::ChatDrop { row: ids[2], above: false });
            this.drop_chat_on_row(ids[0], ids[2], cx);
        })
    });
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    assert_eq!(loaded.len(), 3, "all chats reload");
    let mut sorted = loaded;
    sorted.sort_by_key(|c| std::cmp::Reverse(crate::workspace::sort_order(c)));
    let titles: Vec<String> = sorted.iter().map(|c| c.title.to_string()).collect();
    assert_eq!(titles, vec!["chat-1", "chat-2", "chat-0"], "persisted order reproduces the drag");
}
