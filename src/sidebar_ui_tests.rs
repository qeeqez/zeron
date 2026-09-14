//! Headless UI tests for the shared sidebar row: the settings nav and the
//! chat list must render through the same `NavRow` component, so their rows
//! share geometry and styling. Mount pattern matches `ui_tests.rs`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
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
