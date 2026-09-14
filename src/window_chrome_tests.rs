//! Headless tests for app chrome and lifecycle: the macOS menu bar, window
//! actions (fullscreen/minimize/zoom), the quit gate, and app-level actions
//! that must work with no window open. Same harness as `ui_tests` — a real
//! `Workspace` in a test window, driven by native input events.

use std::any::TypeId;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, OwnedMenuItem, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, AnyWindowHandle, &mut VisualTestContext) {
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
    let handle = cx.update(|window, _| window.window_handle());
    (ws.unwrap(), handle, cx)
}

/// The action type registered on a named menu item, if it is an action item.
fn menu_action(menu: &gpui_kit::OwnedMenu, name: &str) -> Option<TypeId> {
    menu.items.iter().find_map(|item| match item {
        OwnedMenuItem::Action { name: n, action, .. } if n == name => Some(action.as_any().type_id()),
        _ => None,
    })
}

#[test]
fn menu_bar_matches_native_macos_shape() {
    let app = TestAppContext::single();
    app.update(|cx| cx.set_menus(crate::app_menus()));
    let menus = app.read(|cx| cx.get_menus().expect("menus should be installed"));
    let names: Vec<&str> = menus.iter().map(|m| m.name.as_ref()).collect();
    assert_eq!(names, ["Rixl Code", "File", "Edit", "View", "Window"], "menu bar should match the native macOS shape");

    let app_menu = &menus[0];
    assert_eq!(menu_action(app_menu, "About Rixl Code"), Some(TypeId::of::<crate::AboutApp>()));
    assert_eq!(menu_action(app_menu, "Quit Rixl Code"), Some(TypeId::of::<crate::QuitApp>()));
    assert_eq!(menu_action(app_menu, "Hide Rixl Code"), Some(TypeId::of::<crate::HideApp>()));
    assert!(
        app_menu.items.iter().any(|i| matches!(i, OwnedMenuItem::SystemMenu(_))),
        "app menu should include the Services system submenu"
    );

    let edit = &menus[2];
    // The Edit items must dispatch the real input actions — `NoAction`
    // placeholders are permanently disabled by menu validation.
    assert_eq!(menu_action(edit, "Undo"), Some(TypeId::of::<gpui_kit::component::input::Undo>()));
    assert_eq!(menu_action(edit, "Cut"), Some(TypeId::of::<gpui_kit::component::input::Cut>()));
    assert_eq!(menu_action(edit, "Select All"), Some(TypeId::of::<gpui_kit::component::input::SelectAll>()));

    let window_menu = &menus[4];
    assert_eq!(menu_action(window_menu, "Minimize"), Some(TypeId::of::<crate::MinimizeWindow>()));
    assert_eq!(menu_action(window_menu, "Zoom"), Some(TypeId::of::<crate::ZoomWindow>()));
    assert_eq!(menu_action(&menus[3], "Enter Full Screen"), Some(TypeId::of::<crate::EnterFullscreen>()));
}

#[test]
fn fullscreen_action_toggles_window() {
    let mut app = TestAppContext::single();
    let (_ws, handle, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!window.is_fullscreen(), "window should start windowed");
    });
    TestAppContext::dispatch_action(cx, handle, crate::EnterFullscreen);
    cx.update(|window, _| assert!(window.is_fullscreen(), "Enter Full Screen should toggle fullscreen on"));
    TestAppContext::dispatch_action(cx, handle, crate::EnterFullscreen);
    cx.update(|window, _| assert!(!window.is_fullscreen(), "Enter Full Screen should toggle fullscreen off"));
}

#[test]
fn cmd_w_closes_window_through_real_keymap() {
    let mut app = TestAppContext::single();
    app.update(|cx| cx.bind_keys(crate::workspace_keys()));
    let (_ws, _handle, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.press("cmd-w", cx);
    });
    assert_eq!(app.windows().len(), 0, "cmd-w should close the workspace window");
}

#[test]
fn quit_prompts_while_reply_is_running() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        cx.bind_keys(crate::workspace_keys());
        crate::install_app_actions(cx);
    });
    let (ws, handle, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        ws.update(cx, |this, _| this.chats[this.active].running = true);
    });
    TestAppContext::dispatch_action(cx, handle, crate::QuitApp);
    assert!(app.has_pending_prompt(), "quit should confirm while a reply is generating");

    // Cancelling leaves the app running — the window and its chat survive.
    app.simulate_prompt_answer("Cancel");
    app.run_until_parked();
    assert_eq!(app.windows().len(), 1, "cancelled quit must keep the window");
}

#[test]
fn quit_without_running_reply_skips_prompt() {
    let mut app = TestAppContext::single();
    app.update(crate::install_app_actions);
    let (_ws, handle, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    TestAppContext::dispatch_action(cx, handle, crate::QuitApp);
    assert!(!app.has_pending_prompt(), "quit with nothing running should not prompt");
}

#[test]
fn about_action_opens_dialog() {
    let mut app = TestAppContext::single();
    app.update(crate::install_app_actions);
    let (_ws, handle, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear(cx);
    });
    TestAppContext::dispatch_action(cx, handle, crate::AboutApp);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("about-dialog").visible(), "About should open its dialog");
    });
}

#[test]
fn new_window_action_works_with_no_windows() {
    let app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::install_app_actions(cx);
    });
    assert_eq!(app.windows().len(), 0);
    // Menu actions with no open window dispatch to the global listeners.
    app.update(|cx| cx.dispatch_action(&crate::NewWindow));
    app.run_until_parked();
    assert_eq!(app.windows().len(), 1, "New Window should open a workspace window");
}
