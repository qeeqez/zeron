//! Headless tests for chat folders: assigning a chat to a folder, folder
//! persistence, rename/delete reassigning member chats, and the row menu's
//! "Move to folder" submenu + "New folder…" dialog. Same mount harness as
//! `chat_ops_tests.rs` — duplicated because sibling test files can't share
//! private helpers.

use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-folder-test-{}", std::process::id()));
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

#[test]
fn assign_chat_to_folder() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_folder(chat_id, "Work", cx));
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).chats[0].folder, "Work");
        assert_eq!(ws.read(cx).folder_names(), ["Work"]);
    });
    // An empty name unfiles the chat — it falls back to the default group.
    ws.update(cx, |this, cx| this.set_chat_folder(chat_id, "  ", cx));
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).chats[0].folder, "");
        assert!(ws.read(cx).folder_names().is_empty());
    });
}

#[test]
fn folder_survives_save_and_reload() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| {
        this.set_chat_folder(chat_id, "Work", cx);
        this.save();
    });
    cx.update(|_, cx| {
        let dir = ws.read(cx).project.chats_dir();
        let mut next_id = 0;
        let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].folder, "Work", "folder must round-trip through the chat file");
    });
}

#[test]
fn rename_folder_reassigns_its_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        this.new_chat(cx);
        let (a, b) = (this.chats[0].id, this.chats[1].id);
        this.set_chat_folder(a, "Work", cx);
        this.set_chat_folder(b, "Work", cx);
        this.toggle_folder("Work", cx);
        this.rename_folder("Work", "Projects", cx);
    });
    cx.update(|_, cx| {
        let ws = ws.read(cx);
        assert!(ws.chats.iter().all(|c| c.folder == "Projects"), "rename must rewrite every member");
        assert_eq!(ws.folder_names(), ["Projects"]);
        // A collapsed folder stays collapsed under its new name.
        assert!(ws.collapsed_folders.contains("Projects"));
        assert!(!ws.collapsed_folders.contains("Work"));
    });
}

#[test]
fn delete_folder_unfiles_its_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        let id = this.chats[0].id;
        this.set_chat_folder(id, "Work", cx);
        this.delete_folder("Work", cx);
    });
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).chats[0].folder, "", "delete must unfile members");
        assert!(ws.read(cx).folder_names().is_empty());
    });
}

#[test]
fn move_to_folder_submenu_files_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let unfiled = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            let filed = this.chats[0].id;
            this.set_chat_folder(filed, "Work", cx);
            this.chats[1].id
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("chat-row", unfiled), cx);
        window.draw(cx).clear(cx);
        window.click(("chat-menu", unfiled), cx);
        window.draw(cx).clear(cx);
        // Menu order: Pin, Rename, Move to folder, Duplicate, Export, …
        window.within("popup-menu").hover(2usize, cx);
        window.draw(cx).clear(cx);
        // Submenu leads with the existing folders — "Work" is index 0.
        window.within("submenu").click(0usize, cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[1].folder, "Work", "submenu pick should file the chat");
    });
}

#[test]
fn new_folder_dialog_files_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("chat-row", chat_id), cx);
        window.draw(cx).clear(cx);
        window.click(("chat-menu", chat_id), cx);
        window.draw(cx).clear(cx);
        window.within("popup-menu").hover(2usize, cx);
        window.draw(cx).clear(cx);
        // No folders yet: "New folder…" is the only submenu item.
        window.within("submenu").click(0usize, cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_some(), "New folder… should open the dialog");
        ws.read(cx).folder_input.clone().update(cx, |s, cx| s.set_value("Work", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].folder, "Work", "dialog OK should file the chat");
        assert!(window.try_find("dialog").is_none(), "dialog should close on OK");
    });
}
