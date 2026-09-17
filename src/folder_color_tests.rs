//! Headless tests for folder color tags: set/clear through
//! `set_folder_color`, persistence through `ProjectState.folder_colors`,
//! the header's color dot, the member rows' left-edge accent, and the
//! folder menu's Color submenu (checked state + picks). Same mount harness
//! as `folder_tests.rs` — duplicated because sibling test files can't
//! share private helpers.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{Chat, ChatColor};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-foldercolor-test-{}", std::process::id()));
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

/// File a chat under `folder` and return its id.
fn file_chat(ws: &Entity<Workspace>, cx: &mut VisualTestContext, folder: &str) -> u64 {
    ws.update(cx, |this, cx| {
        let id = this.chats[0].id;
        this.set_chat_folder(id, folder, cx);
        id
    })
}

/// Open the folder header's Color submenu. The context menu builds in a
/// deferred callback, so the right-click and the hover sit in separate
/// updates (same pattern as `right_click_opens_same_menu`).
fn open_folder_color_submenu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click("group-header-Work", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Menu order: Rename folder, Color, Delete folder.
        window.within("popup-menu").hover(1usize, cx);
        window.draw(cx).clear(cx);
    });
}

#[test]
fn set_and_clear_folder_color() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    file_chat(&ws, cx, "Work");
    ws.update(cx, |this, cx| this.set_folder_color("Work", Some(ChatColor::Blue), cx));
    cx.update(|_, cx| assert_eq!(ws.read(cx).folder_color("Work"), Some(ChatColor::Blue)));
    ws.update(cx, |this, cx| this.set_folder_color("Work", None, cx));
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).folder_color("Work"), None);
        assert!(ws.read(cx).folder_colors.is_empty(), "clearing drops the map entry");
    });
}

#[test]
fn folder_color_survives_save_and_reload() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    file_chat(&ws, cx, "Work");
    ws.update(cx, |this, cx| this.set_folder_color("Work", Some(ChatColor::Purple), cx));
    cx.update(|_, cx| {
        let state = ws.read(cx).project.load_state();
        assert_eq!(state.folder_colors.get("Work").map(String::as_str), Some("purple"), "the tag must reach state.json");
    });
    // Clearing the tag persists too — the key leaves state.json.
    ws.update(cx, |this, cx| this.set_folder_color("Work", None, cx));
    cx.update(|_, cx| {
        let state = ws.read(cx).project.load_state();
        assert!(!state.folder_colors.contains_key("Work"), "a cleared tag must not resurrect");
    });
}

#[test]
fn folder_color_loads_from_project_state() {
    // A previous session's store: one chat filed under "Work", the folder
    // tagged green — both must be live after launch. `mount` can't be
    // reused: it re-points HOME, so this test mounts inline after seeding.
    let dir = std::env::temp_dir().join(format!("rixlcode-foldercolor-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    let project = crate::project::Project::current();
    let mut chat = Chat::new(0, "filed");
    chat.folder = "Work".into();
    crate::persist::save_chats(&project.chats_dir(), &[chat]);
    project.save_state(&crate::project::ProjectState {
        folder_colors: [("Work".to_string(), "green".to_string())].into_iter().collect(),
        ..Default::default()
    });

    let mut app = TestAppContext::single();
    app.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = app.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    let ws = ws.unwrap();
    cx.update(|_, cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.folder_color("Work"), Some(ChatColor::Green), "the persisted tag must load");
        assert_eq!(ws.folder_names(), ["Work"]);
    });
}

#[test]
fn folder_header_shows_color_dot() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    file_chat(&ws, cx, "Work");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("group-header-Work").visible(), "the folder header renders");
        assert!(window.try_find("folder-color-dot-Work").is_none(), "untagged headers have no dot");
    });
    ws.update(cx, |this, cx| this.set_folder_color("Work", Some(ChatColor::Green), cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("folder-color-dot-Work").visible(), "tagged headers carry the color dot");
    });
    ws.update(cx, |this, cx| this.set_folder_color("Work", None, cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("folder-color-dot-Work").is_none(), "clearing the tag removes the dot");
    });
}

#[test]
fn member_rows_show_folder_edge() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (filed, unfiled) = ws.update(cx, |this, cx| {
        this.new_chat(cx);
        let (filed, unfiled) = (this.chats[0].id, this.chats[1].id);
        this.set_chat_folder(filed, "Work", cx);
        (filed, unfiled)
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(format!("chat-row-{filed}-folder-edge")).is_none(), "untagged folders give no accent");
    });
    ws.update(cx, |this, cx| this.set_folder_color("Work", Some(ChatColor::Orange), cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(format!("chat-row-{filed}-folder-edge")).visible(), "member rows get the folder accent");
        assert!(window.try_find(format!("chat-row-{unfiled}-folder-edge")).is_none(), "unfiled rows stay unmarked");
    });
    ws.update(cx, |this, cx| this.set_folder_color("Work", None, cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(format!("chat-row-{filed}-folder-edge")).is_none(), "clearing removes the accent");
    });
}

#[test]
fn folder_color_submenu_checks_current() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    file_chat(&ws, cx, "Work");
    ws.update(cx, |this, cx| this.set_folder_color("Work", Some(ChatColor::Blue), cx));
    open_folder_color_submenu(cx);
    cx.update(|window, _cx| {
        assert!(window.within("submenu").find("color-swatch-blue").visible(), "the submenu lists the swatches");
        assert_eq!(window.within("submenu").find("color-swatch-blue").checked(), Some(true), "the current tag reads checked");
        assert_eq!(window.within("submenu").find("color-swatch-red").checked(), Some(false), "other swatches read unchecked");
    });
}

#[test]
fn folder_color_submenu_pick_tags_and_none_clears() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    file_chat(&ws, cx, "Work");
    open_folder_color_submenu(cx);
    cx.update(|window, cx| {
        // Swatch order follows ChatColor::ALL — Green is index 3.
        window.within("submenu").click(3usize, cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).folder_color("Work"), Some(ChatColor::Green), "a swatch pick tags the folder");
    });
    open_folder_color_submenu(cx);
    cx.update(|window, cx| {
        // "None" trails the six swatches.
        window.within("submenu").click(6usize, cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).folder_color("Work"), None, "None clears the tag");
    });
}

#[test]
fn rename_folder_keeps_its_color() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    file_chat(&ws, cx, "Work");
    ws.update(cx, |this, cx| {
        this.set_folder_color("Work", Some(ChatColor::Red), cx);
        this.rename_folder("Work", "Projects", cx);
    });
    cx.update(|_, cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.folder_color("Projects"), Some(ChatColor::Red), "the tag follows the rename");
        assert_eq!(ws.folder_color("Work"), None, "the old name drops the tag");
    });
}
