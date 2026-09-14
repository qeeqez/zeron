//! Headless UI tests for the Appearance settings section — font pickers,
//! size steppers, code preview, contrast slider and the frosted-sidebar
//! toggle. Mount pattern matches `ui_tests.rs`.
//!
//! Events emitted inside `cx.update` flush when that update returns, so each
//! action and its assertions run in separate `cx.update` blocks.

use gpui_kit::component::Root;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::component::slider::SliderValue;
use gpui_kit::component::theme::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, point, px};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-appearance-test-{}", std::process::id()));
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

/// Open settings and switch to the Appearance section.
fn open_appearance(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-appearance", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-appearance").visible());
    });
}

#[test]
fn appearance_section_shows_font_controls_and_preview() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    open_appearance(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        for id in ["font-select", "code-font-select", "code-preview", "contrast-slider", "toggle-sidebar-frosted"] {
            assert!(window.try_find(id).is_some(), "{id} should render in Appearance");
        }
    });
}

#[test]
fn code_font_and_size_apply_and_persist() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_appearance(cx);
    // Pick a code font through the picker's Confirm event — the same path a
    // dropdown selection takes.
    cx.update(|_window, cx| {
        let select = ws.read(cx).settings_panel.read(cx).code_font_select.clone();
        select.update(cx, |_, cx| cx.emit(SelectEvent::<Vec<String>>::Confirm(Some("Test Mono".to_string()))));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).code_font_family, "Test Mono");
        assert_eq!(cx.global::<Theme>().mono_font_family.as_ref(), "Test Mono");
        assert_eq!(crate::persist::load_settings().code_font_family, "Test Mono");

        // Code size stepper: +1 then −2.
        window.click("code-font-select-inc", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).code_font_size, 14);
        assert_eq!(f32::from(cx.global::<Theme>().mono_font_size), 14.);

        window.click("code-font-select-dec", cx);
        window.click("code-font-select-dec", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).code_font_size, 12);
        assert_eq!(crate::persist::load_settings().code_font_size, 12);
    });
}

#[test]
fn interface_font_and_size_apply_and_persist() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_appearance(cx);
    cx.update(|_window, cx| {
        let select = ws.read(cx).settings_panel.read(cx).font_select.clone();
        select.update(cx, |_, cx| cx.emit(SelectEvent::<Vec<String>>::Confirm(Some("Test Sans".to_string()))));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_family, "Test Sans");
        assert_eq!(cx.global::<Theme>().font_family.as_ref(), "Test Sans");
        assert_eq!(crate::persist::load_settings().font_family, "Test Sans");

        // Interface size drives theme.font_size (the rem base) too.
        window.click("font-select-inc", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_size, 15);
        assert_eq!(f32::from(cx.global::<Theme>().font_size), 15.);
        assert_eq!(crate::persist::load_settings().font_size, 15);
    });
    // Clearing the picker (Confirm(None)) restores the system default.
    cx.update(|_window, cx| {
        let select = ws.read(cx).settings_panel.read(cx).font_select.clone();
        select.update(cx, |_, cx| cx.emit(SelectEvent::<Vec<String>>::Confirm(None)));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_family, "");
        assert_eq!(cx.global::<Theme>().font_family.as_ref(), ".SystemUIFont");
    });
}

#[test]
fn contrast_slider_clamps_and_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_appearance(cx);
    let (w, h) = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let bounds = window.find("contrast-slider").bounds();
        (f32::from(bounds.size.width), f32::from(bounds.size.height))
    });
    // Click at the far right → 200 (slider clamps to its max).
    cx.update(|window, cx| {
        window.click_at("contrast-slider", point(px(w - 1.), px(h / 2.)), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).contrast, 200, "right-edge click should max out contrast");
        assert_eq!(crate::persist::load_settings().contrast, 200);
    });
    // Click at the far left → 50.
    cx.update(|window, cx| {
        window.click_at("contrast-slider", point(px(0.), px(h / 2.)), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).contrast, 50, "left-edge click should bottom out contrast");
        assert_eq!(crate::persist::load_settings().contrast, 50);

        // The slider state itself stays inside [50, 200].
        let slider = ws.read(cx).settings_panel.read(cx).contrast_slider.clone();
        let value = slider.read(cx).value();
        assert!(matches!(value, SliderValue::Single(v) if (50. ..=200.).contains(&v)));
    });
}

#[test]
fn sidebar_frosted_toggle_flips_rendering_mode() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).sidebar_frosted, "frosted sidebar is the default");
    });
    open_appearance(cx);
    cx.update(|window, cx| {
        window.click("toggle-sidebar-frosted", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).sidebar_frosted, "toggle should switch to opaque");
        assert!(!crate::persist::load_settings().sidebar_frosted, "toggle should persist");
        assert!(window.find("sidebar-wrap").visible(), "sidebar still renders opaque");

        window.click("toggle-sidebar-frosted", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).sidebar_frosted, "toggle back restores frosted glass");
        assert!(crate::persist::load_settings().sidebar_frosted);
    });
}
