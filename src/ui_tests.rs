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

use gpui_kit::component::theme::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

#[test]
fn sidebar_toggle_hides_and_restores() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Sidebar starts expanded — its wrap element is present.
        assert!(window.find("sidebar-wrap").visible(), "sidebar should start visible");

        // The floating toggle (right of the traffic lights) collapses it fully —
        // Offcanvas removes it from layout rather than shrinking to an icon rail.
        window.click("collapse", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("sidebar-wrap").is_none(), "sidebar should hide completely");

        // The toggle stays mounted and re-opens the sidebar.
        window.click("collapse", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("sidebar-wrap").visible(), "sidebar should re-open");
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
