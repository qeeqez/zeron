//! Headless tests for sidebar chat management: the hover-revealed "…" row
//! menu and the inline rename editor (Enter commits, Escape cancels, a click
//! outside commits).

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-chatops-test-{}", std::process::id()));
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
fn row_menu_button_reveals_on_hover() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-row", chat_id)).visible(), "chat row should render");
        // The "…" affordance exists but stays hidden until the row is hovered.
        assert!(!window.find(("chat-menu", chat_id)).visible(), "menu button hidden before hover");
        window.hover(("chat-row", chat_id), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-menu", chat_id)).visible(), "menu button reveals on hover");
    });
}

#[test]
fn row_menu_button_opens_dropdown() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("chat-row", chat_id), cx);
        window.draw(cx).clear(cx);
        window.click(("chat-menu", chat_id), cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "clicking … should open the chat menu");
        // The menu button stays visible while its menu is open.
        assert!(window.find(("chat-menu", chat_id)).visible(), "menu button stays while open");
    });
}

#[test]
fn right_click_opens_same_menu() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("chat-row", chat_id), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the chat menu");
    });
}

#[test]
fn inline_rename_commits_on_enter() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| {
        this.chats[0].title = "Old title".into();
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("chat-row", chat_id), cx);
        window.draw(cx).clear(cx);
        window.click(("chat-menu", chat_id), cx);
        window.draw(cx).clear(cx);
        // Menu order: Pin, Rename, Duplicate, Export, Delete, Archive.
        window.within("popup-menu").click(1usize, cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).renaming, Some(chat_id), "Rename should start inline edit");
        assert!(window.find(("rename-input", chat_id)).visible(), "editor should replace the title");
    });
    // The deferred focus lands between updates; typing then replaces the
    // selected title.
    cx.update(|window, cx| {
        window.input("Renamed chat", cx);
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].title.as_ref(), "Renamed chat");
        assert_eq!(ws.read(cx).renaming, None, "Enter should end the edit");
        assert!(window.try_find(("rename-input", chat_id)).is_none(), "editor should unmount");
    });
}

#[test]
fn inline_rename_escape_cancels() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| {
        this.chats[0].title = "Keep me".into();
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| this.start_inline_rename(0, window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find(("rename-input", chat_id)).visible());
    });
    cx.update(|window, cx| {
        window.input("Discard", cx);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].title.as_ref(), "Keep me", "Escape must not rename");
        assert_eq!(ws.read(cx).renaming, None, "Escape should end the edit");
    });
}

#[test]
fn inline_rename_commits_when_another_row_is_clicked() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| this.new_chat(cx));
    let (id0, id1) = cx.update(|_, cx| (ws.read(cx).chats[0].id, ws.read(cx).chats[1].id));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        ws.update(cx, |this, cx| this.start_inline_rename(0, window, cx));
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).renaming, Some(id0));
    });
    cx.update(|window, cx| {
        window.input("First chat", cx);
        // Clicking the other row commits the edit, then selects that row.
        window.click(("chat-row", id1), cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].title.as_ref(), "First chat");
        assert_eq!(ws.read(cx).renaming, None);
        assert_eq!(ws.read(cx).active, 1, "the clicked row should become active");
    });
}
