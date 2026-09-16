//! Headless tests for chat color tags: set/clear through `set_chat_color`,
//! persistence round-trips, the sidebar row's dot, the titlebar dot, and the
//! ⋯ menu's Color submenu (checked state + picks). Same mount harness as
//! `chat_ops_tests.rs` — duplicated because sibling test files can't share
//! private helpers.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::model::{Chat, ChatColor};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-chatcolor-test-{}", std::process::id()));
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

/// A fresh throwaway chats dir — `save_chats`/`load_chats` take the dir
/// explicitly, so these tests never touch the real profile.
fn temp_chats_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-chatcolor-persist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Open the ⋯ menu's Color submenu; the caller draws once more before
/// clicking inside it.
fn open_color_submenu(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.draw(cx).clear(cx);
    window.click("chat-menu", cx);
    window.draw(cx).clear(cx);
    // Menu order: New Temporary Chat, Pin, Rename, Color, Export, …
    window.within("popup-menu").hover(3usize, cx);
    window.draw(cx).clear(cx);
}

#[test]
fn set_and_clear_chat_color() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, Some(ChatColor::Blue), cx));
    cx.update(|_, cx| assert_eq!(ws.read(cx).chats[0].color, Some(ChatColor::Blue)));
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, None, cx));
    cx.update(|_, cx| assert_eq!(ws.read(cx).chats[0].color, None));
}

#[test]
fn color_survives_save_and_reload() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, Some(ChatColor::Purple), cx));
    cx.update(|_, cx| {
        let dir = ws.read(cx).project.chats_dir();
        let mut next_id = 0;
        let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].color, Some(ChatColor::Purple), "color must round-trip through the chat file");
    });
    // Clearing the tag persists too — the reloaded chat is untagged.
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, None, cx));
    cx.update(|_, cx| {
        let dir = ws.read(cx).project.chats_dir();
        let mut next_id = 0;
        let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
        assert_eq!(loaded[0].color, None, "a cleared tag must not resurrect");
    });
}

#[test]
fn unknown_color_name_loads_untagged() {
    let dir = temp_chats_dir("unknown");
    std::fs::write(dir.join("0.json"), r#"{"v":1,"title":"t","messages":[],"color":"magenta"}"#).unwrap();
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, false);
    assert_eq!(loaded.len(), 1, "an unknown color name must not drop the chat");
    assert_eq!(loaded[0].color, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sidebar_row_shows_color_dot() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("chat-color-dot", chat_id)).is_none(), "untagged rows have no dot");
    });
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, Some(ChatColor::Green), cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let dot = window.find(("chat-color-dot", chat_id));
        assert!(dot.visible(), "tagged rows carry the color dot");
    });
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, None, cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("chat-color-dot", chat_id)).is_none(), "clearing the tag removes the dot");
    });
}

#[test]
fn titlebar_shows_color_dot() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("chat-color-dot").is_none(), "untagged chats show no titlebar dot");
    });
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, Some(ChatColor::Orange), cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("chat-color-dot").visible(), "the titlebar shows the tag dot");
    });
}

#[test]
fn color_submenu_checks_current_color() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    ws.update(cx, |this, cx| this.set_chat_color(chat_id, Some(ChatColor::Blue), cx));
    cx.update(|window, cx| {
        open_color_submenu(window, cx);
        assert!(window.within("submenu").find("color-swatch-blue").visible(), "the submenu lists the swatches");
        assert_eq!(window.within("submenu").find("color-swatch-blue").checked(), Some(true), "the current tag reads checked");
        assert_eq!(window.within("submenu").find("color-swatch-red").checked(), Some(false), "other swatches read unchecked");
    });
}

#[test]
fn color_submenu_pick_tags_and_none_clears() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_color_submenu(window, cx);
        // Swatch order follows ChatColor::ALL — Green is index 3.
        window.within("submenu").click(3usize, cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].color, Some(ChatColor::Green), "a swatch pick tags the chat");
    });
    cx.update(|window, cx| {
        open_color_submenu(window, cx);
        // "None" trails the six swatches.
        window.within("submenu").click(6usize, cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].color, None, "None clears the tag");
    });
}

#[test]
fn chat_menu_offers_color_submenu() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(
            snapshots(window).iter().any(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some("Color")),
            "chat menu should offer the Color submenu"
        );
    });
}

#[test]
fn temp_chat_color_never_reaches_disk() {
    let dir = temp_chats_dir("temp");
    let mut temp = Chat::new(1, "temp");
    temp.ephemeral = true;
    temp.color = Some(ChatColor::Red);
    crate::persist::save_chats(&dir, &[Chat::new(0, "normal"), temp]);
    assert!(dir.join("0.json").exists(), "the normal chat persists");
    assert!(!dir.join("1.json").exists(), "a tagged temporary chat still writes no file");
    let _ = std::fs::remove_dir_all(&dir);
}
