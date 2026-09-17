//! Headless tests for the split view: the row menu's "Open in Split" puts
//! a second chat beside the active one, the pane's titlebar click swaps
//! the panes, × clears it, and deleting or archiving the split chat drops
//! the pane. The composer stays bound to the active chat — the split pane
//! is a view, never an edit target. Mount pattern matches
//! `select_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-split-test-{}", std::process::id()));
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

/// `n` extra chats on top of the one the workspace opens with — returns
/// every chat id in `chats` order.
fn chats(ws: &Entity<Workspace>, n: usize, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            for _ in 0..n {
                this.new_chat(cx);
            }
            this.chats.iter().map(|c| c.id).collect()
        })
    })
}

/// "Open in Split" puts that chat in the pane — the titlebar and close
/// button render, and the active chat is untouched.
#[test]
fn open_split_shows_second_pane() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    // chats: [0]=first, [1], [2]=active (new_chat selects each new one).
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            let ix = this.chat_index(ids[0]).unwrap();
            this.open_split(ix, cx);
        });
        let (secondary, active) = (ws.read(cx).secondary, ws.read(cx).active);
        assert_eq!(secondary, Some(0));
        assert_eq!(active, 2);
        window.draw(cx).clear(cx);
        assert!(window.try_find("split-titlebar").is_some(), "split pane renders");
        assert!(window.try_find("split-close").is_some(), "close button renders");
    });
}

/// Opening the active chat — or re-opening the split one — is a no-op.
#[test]
fn open_split_noops_on_visible_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.open_split(this.active, cx);
            assert_eq!(this.secondary, None, "active chat can't split");
            this.open_split(this.chat_index(ids[0]).unwrap(), cx);
            this.open_split(this.chat_index(ids[0]).unwrap(), cx);
            assert_eq!(this.secondary, Some(0), "re-open keeps the pane");
        })
    });
}

/// Clicking the split pane's titlebar swaps the panes: its chat becomes
/// active and the old active takes the split slot.
#[test]
fn titlebar_click_swaps_panes() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_split(this.chat_index(ids[0]).unwrap(), cx));
        window.draw(cx).clear(cx);
        window.click("split-titlebar", cx);
        let (secondary, active) = (ws.read(cx).secondary, ws.read(cx).active);
        assert_eq!(active, 0, "split chat became active");
        assert_eq!(secondary, Some(2), "old active took the split slot");
    });
}

/// The × on the split titlebar clears the pane without touching the chat.
#[test]
fn close_split_clears_pane() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_split(this.chat_index(ids[0]).unwrap(), cx));
        window.draw(cx).clear(cx);
        window.click("split-close", cx);
        let (secondary, active, len) = (ws.read(cx).secondary, ws.read(cx).active, ws.read(cx).chats.len());
        assert_eq!(secondary, None, "pane closed");
        assert_eq!(active, 2, "active unchanged");
        assert_eq!(len, 3, "the chat survives the close");
        window.draw(cx).clear(cx);
        assert!(window.try_find("split-titlebar").is_none(), "pane unmounted");
    });
}

/// Deleting the split chat clears the pane; deleting an earlier chat
/// shifts the pane's index so it still tracks the same chat.
#[test]
fn deleted_secondary_clears() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.open_split(this.chat_index(ids[0]).unwrap(), cx);
            this.delete_chat_now(0, window, cx);
            assert_eq!(this.secondary, None, "deleting the split chat closes the pane");
        })
    });
}

/// Removing a chat *before* the split one shifts the pane's index — it
/// still tracks the same chat, not the same position.
#[test]
fn delete_before_secondary_shifts_index() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.open_split(this.chat_index(ids[1]).unwrap(), cx);
            this.delete_chat_now(0, window, cx);
            assert_eq!(this.secondary, Some(0), "index shifted down");
            assert_eq!(this.chats[0].id, ids[1], "the pane still shows the same chat");
        })
    });
}

/// Archiving the split chat clears the pane — archived chats leave the
/// sidebar, so they leave the split too.
#[test]
fn archived_secondary_clears() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.open_split(this.chat_index(ids[0]).unwrap(), cx);
            this.toggle_archive(0, window, cx);
            assert_eq!(this.secondary, None, "archived chat leaves the pane");
        })
    });
}

/// The composer edits the active chat only: text typed while a split is
/// open lands in the active chat's draft on switch, and the split chat's
/// draft stays empty.
#[test]
fn composer_edits_active_only() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.open_split(this.chat_index(ids[0]).unwrap(), cx);
            this.composer.update(cx, |s, cx| s.set_value("draft for active", window, cx));
            // Switching chats saves the composer into the outgoing chat's
            // draft — the split chat's stays empty.
            this.select_chat(this.chat_index(ids[1]).unwrap(), window, cx);
            let split_ix = this.chat_index(ids[0]).unwrap();
            assert_eq!(this.chats[split_ix].draft, "", "split chat's draft untouched");
            assert_eq!(this.chats[2].draft, "draft for active", "outgoing active kept its draft");
        })
    });
}
