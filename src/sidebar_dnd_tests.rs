//! Headless UI tests for dragging a chat row onto a folder group header.
//! `window.drag_to` drives real mouse events through the toolkit's drag
//! threshold, so the assertions cover the whole path: row drag source →
//! `ChatDrag` payload → header drop target → `set_chat_folder`. Mount pattern
//! matches `sidebar_ui_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::{RenameMode, Workspace};

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-dnd-test-{}", std::process::id()));
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

/// Two chats, the first filed under `folder`. Returns (filed, unfiled) ids.
fn two_chats(ws: &Entity<Workspace>, folder: &str, cx: &mut VisualTestContext) -> (u64, u64) {
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            let filed = this.chats[0].id;
            this.set_chat_folder(filed, folder, cx);
            (filed, this.chats[1].id)
        })
    })
}

/// Dragging an unfiled chat onto a folder header files it there.
#[test]
fn drag_chat_row_onto_folder_header_files_it() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (filed, unfiled) = two_chats(&ws, "Work", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.drag_to(("chat-row", unfiled), "group-header-Work", cx);
        window.draw(cx).clear(cx);
        let chats = &ws.read(cx).chats;
        assert_eq!(chats.iter().find(|c| c.id == unfiled).unwrap().folder, "Work", "drop files the chat");
        assert_eq!(chats.iter().find(|c| c.id == filed).unwrap().folder, "Work", "existing member untouched");
        // The moved row now renders inside the folder group, above Unfiled.
        let row = window.find(("chat-row", unfiled));
        let header = window.find("group-header-Work");
        assert!(row.visible(), "moved chat still renders");
        assert!(row.bounds().origin.y > header.bounds().origin.y, "row sits under its folder header");
    });
}

/// Dragging a filed chat onto the "Unfiled" header unfiles it.
#[test]
fn drag_chat_row_onto_unfiled_header_clears_folder() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (filed, _) = two_chats(&ws, "Work", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.drag_to(("chat-row", filed), "group-header-Unfiled", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats.iter().find(|c| c.id == filed).unwrap().folder, "", "Unfiled drop clears the folder");
        assert!(window.try_find("group-header-Work").is_none(), "empty folder group disappears");
        assert!(window.find(("chat-row", filed)).visible(), "chat still renders under Unfiled");
    });
}

/// Filing is orthogonal to pinning: a pinned chat dropped into a folder keeps
/// its pin and renders inside the folder group, not the Pinned bucket.
#[test]
fn drag_pinned_chat_into_folder_keeps_pin() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (_, unfiled) = two_chats(&ws, "Work", cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            let ix = this.chats.iter().position(|c| c.id == unfiled).unwrap();
            this.toggle_pin(ix, cx);
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.drag_to(("chat-row", unfiled), "group-header-Work", cx);
        window.draw(cx).clear(cx);
        let (pinned, folder) = {
            let chat = ws.read(cx).chats.iter().find(|c| c.id == unfiled).unwrap();
            (chat.pinned, chat.folder.clone())
        };
        assert!(pinned, "drop keeps the pin");
        assert_eq!(folder, "Work", "drop still files the chat");
        let row = window.find(("chat-row", unfiled));
        let header = window.find("group-header-Work");
        assert!(row.bounds().origin.y > header.bounds().origin.y, "pinned chat renders inside the folder group");
    });
}

/// The drag threshold protects the row's other gestures: a plain click still
/// selects, a double-click still opens the inline rename editor.
#[test]
fn click_and_double_click_survive_drag_source() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (_, unfiled) = two_chats(&ws, "Work", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("chat-row", unfiled), cx);
        assert_eq!(ws.read(cx).chats[ws.read(cx).active].id, unfiled, "click still selects the chat");

        window.double_click(("chat-row", unfiled), cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).renaming, Some(unfiled), "double-click still starts rename");
        assert_eq!(ws.read(cx).rename_mode, RenameMode::Inline, "rename stays inline");
    });
}
