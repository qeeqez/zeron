//! Headless UI tests via `gpui_kit::test` (test-support feature). These mount
//! the real `Workspace` in a test window and drive it with native input events
//! — no pixel capture, no display.
//!
//! `#[gpui_kit::test]` and `use gpui_kit::*` both crash the proc-macro on this
//! nightly, so tests use `TestAppContext::single()` under plain `#[test]` and
//! import only what they need.
//!
//! `dirs_home` reads `HOME`, so each test points it at a temp dir to keep the
//! real `~/.rixl/rixlcode` untouched.

use gpui_kit::component::Root;
use gpui_kit::component::theme::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, InteractiveElement, ParentElement, Role, TestAppContext, TestSupportExt, VisualTestContext, div};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    // The real window wraps Workspace in gpui_component::Root — sheets,
    // dialogs and notifications only render when the root is a Root.
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
fn sidebar_toggle_hides_and_restores() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Sidebar starts expanded — its wrap element is present.
        assert!(window.find("sidebar-wrap").visible(), "sidebar should start visible");

        // The sidebar is full-height: its top drag strip sits at the window's
        // top edge, and the content's titlebar is the same height — together
        // they read as one continuous titlebar row. The toggle is a fixed
        // overlay right of the traffic lights, present in both states.
        let sidebar_strip = window.find("sidebar-titlebar");
        assert_eq!(f32::from(sidebar_strip.bounds().origin.y), 0.0, "sidebar top strip must reach the window top");
        let open_bounds = window.find("sidebar-toggle").bounds();

        // Clicking the toggle collapses the sidebar fully — Offcanvas removes
        // it from layout rather than shrinking to an icon rail.
        window.click("sidebar-toggle", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("sidebar-wrap").is_none(), "sidebar should hide completely");

        // Collapsed: the toggle is a fixed overlay — same spot, now over the
        // content's titlebar.
        assert_eq!(window.find("sidebar-toggle").bounds(), open_bounds, "toggle must stay right of the traffic lights in both states");

        // The toggle stays clickable in the collapsed state and re-opens the
        // sidebar.
        window.click("sidebar-toggle", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("sidebar-wrap").visible(), "sidebar should re-open");
        window.find("sidebar-toggle");
    });
}

#[test]
fn theme_setting_applies_and_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Drive the same path the Appearance buttons use: set theme, apply, save.
        for (setting, expect_dark) in [("dark", true), ("light", false)] {
            ws.update(cx, |this, cx| {
                this.theme = setting.to_string();
                this.apply_theme(window, cx);

                this.save_settings();
            });
            let dark = cx.global::<Theme>().mode.is_dark();
            assert_eq!(dark, expect_dark, "theme {setting} should apply");
            assert_eq!(crate::persist::load_settings().theme, setting, "theme {setting} should persist");
        }
    });
}

#[test]
fn settings_gear_opens_settings_screen() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new("escape", crate::EscapeKey, Some("workspace"))]);
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-screen").visible(), "settings screen should open");
        assert!(ws.read(cx).settings_open);

        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("settings-screen").is_none(), "esc should close settings");
        assert!(!ws.read(cx).settings_open);
    });
}

#[test]
fn settings_nav_swaps_sections() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-general").visible(), "general opens by default");
        assert!(window.try_find("settings-section-appearance").is_none());

        window.click("settings-nav-appearance", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-appearance").visible(), "appearance section should show");
        assert!(window.try_find("settings-section-general").is_none(), "general should unmount");

        window.click("settings-nav-shortcuts", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-shortcuts").visible(), "shortcuts section should show");
    });
}

#[test]
fn settings_theme_cards_apply_and_persist() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-appearance", cx);
        window.draw(cx).clear(cx);

        for (card, expect_dark) in [("theme-dark", true), ("theme-light", false)] {
            window.click(card, cx);
            window.draw(cx).clear(cx);
            assert_eq!(cx.global::<Theme>().mode.is_dark(), expect_dark, "{card} should apply");
            assert_eq!(crate::persist::load_settings().theme, &card[6..], "{card} should persist");
        }
    });
}

/// Regression: gpui-component's Root only stores sheet/dialog state — the
/// workspace must mount the render layers or open_sheet/open_dialog draw
/// nothing (this is what made the settings gear appear dead).
#[test]
fn sheet_and_dialog_layers_render() {
    use gpui_kit::component::WindowExt;
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.open_sheet(cx, |sheet, _window, _cx| sheet.title("T").child(div().id("sheet-marker").test_support().child("hi")));
        window.draw(cx).clear(cx);
        assert!(window.find("sheet-marker").visible(), "sheet layer should render");
        window.close_sheet(cx);

        window.open_dialog(cx, |dialog, _window, _cx| dialog.child(div().id("dialog-marker").test_support().child("hi")));
        window.draw(cx).clear(cx);
        assert!(window.find("dialog-marker").visible(), "dialog layer should render");
    });
}

/// Closing settings by pointer: expanded sidebar → the nav rail's "Back to
/// app" row (no duplicate header control); collapsed sidebar → no nav rail,
/// so the header's own close button must be there instead.
#[test]
fn settings_close_control_matches_sidebar_state() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-screen").visible());
        assert!(window.find("settings-back").visible(), "nav back row should show with the sidebar open");
        assert!(window.try_find("settings-close").is_none(), "no header close when the nav row exists");

        window.click("settings-back", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).settings_open, "back row should close settings");
        assert!(window.try_find("settings-screen").is_none());
    });

    // Collapse the sidebar, then open settings the way Cmd-,/menu does.
    cx.update(|window, cx| {
        window.click("sidebar-toggle", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("sidebar-wrap").is_none());
        ws.update(cx, |this, cx| this.open_settings(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("settings-screen").visible());
        assert!(window.try_find("settings-back").is_none(), "no nav rail when collapsed");
        assert!(window.find("settings-close").visible(), "header close must exist when collapsed");

        window.click("settings-close", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).settings_open, "header close should close settings");
        assert!(window.try_find("settings-screen").is_none());
    });
}

/// The General section's boolean rows are real `Switch` controls: role +
/// toggled state come from the element, and a click writes the workspace
/// flag and persists it.
#[test]
fn settings_switches_toggle_workspace_flags() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-general").visible());
        for (id, on) in [("toggle-notify", ws.read(cx).notify_on_done), ("toggle-wrap", ws.read(cx).word_wrap)] {
            let toggle = window.find(id);
            assert_eq!(toggle.role(), Some(Role::Switch), "{id} must render a Switch");
            assert_eq!(toggle.checked(), Some(on), "{id} must mirror the workspace flag");
        }
        window.click("toggle-notify", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).notify_on_done, "switch click should clear the flag");
        assert!(!crate::persist::load_settings().notify_on_done, "switch click should persist");
        assert_eq!(window.find("toggle-notify").checked(), Some(false));
    });
}
