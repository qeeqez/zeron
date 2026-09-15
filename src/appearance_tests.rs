//! Headless UI tests for the Appearance settings section — font pickers,
//! size steppers, code preview, contrast slider and the frosted-sidebar
//! toggle. Mount pattern matches `ui_tests.rs`.
//!
//! Events emitted inside `cx.update` flush when that update returns, so each
//! action and its assertions run in separate `cx.update` blocks.

use gpui_kit::component::Root;
use gpui_kit::component::select::{SearchableVec, SelectEvent};
use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::component::theme::{Theme, ThemeColor};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, Fill, Role, Styled, TestAppContext, VisualTestContext, Window, point, px, transparent_black};

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
        // Mirror `open_workspace_window`: the frosted sidebar strips Root's
        // opaque fill at creation so the blurred window shows through.
        let mut root = Root::new(view, window, cx);
        root.style().background = crate::appearance::frosted_root_background(ws.as_ref().unwrap().read(cx).sidebar_frosted);
        root
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
        select.update(cx, |_, cx| cx.emit(SelectEvent::<SearchableVec<String>>::Confirm(Some("Test Mono".to_string()))));
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
        select.update(cx, |_, cx| cx.emit(SelectEvent::<SearchableVec<String>>::Confirm(Some("Test Sans".to_string()))));
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
        select.update(cx, |_, cx| cx.emit(SelectEvent::<SearchableVec<String>>::Confirm(None)));
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

        // The mechanism, not just the flag: the blurred window background
        // only shows through where pixels are transparent, so the Root
        // layer's fill must be transparent and the sidebar's fill
        // translucent — not an opaque dark color.
        let root_bg = Root::update(window, cx, |root, _, _| root.style().background.clone());
        assert_eq!(root_bg, Some(Fill::from(transparent_black())), "frosted needs a transparent Root fill");
        let fill = crate::appearance::sidebar_fill(cx.global::<Theme>(), true);
        assert!(fill.a > 0. && fill.a < 1., "frosted sidebar fill must be translucent, got alpha {}", fill.a);
        assert!(fill.a >= 0.25, "frosted fill must stay legible over the blur, got alpha {}", fill.a);
        assert_eq!(
            crate::appearance::window_background_appearance(true),
            gpui_kit::WindowBackgroundAppearance::Blurred,
            "frosted needs a blurred window behind the translucent fill"
        );
    });
    open_appearance(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find("toggle-sidebar-frosted").role(), Some(Role::Switch), "frosted row must render a Switch");
        assert_eq!(window.find("toggle-sidebar-frosted").checked(), Some(true), "frosted is on by default");
        window.click("toggle-sidebar-frosted", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).sidebar_frosted, "toggle should switch to opaque");
        assert!(!crate::persist::load_settings().sidebar_frosted, "toggle should persist");
        assert!(window.find("sidebar-wrap").visible(), "sidebar still renders opaque");

        // Opaque mode restores the Root fill and a fully opaque sidebar.
        let root_bg = Root::update(window, cx, |root, _, _| root.style().background.clone());
        assert_eq!(root_bg, None, "unfrosted restores the stock Root fill");
        let fill = crate::appearance::sidebar_fill(cx.global::<Theme>(), false);
        assert_eq!(fill.a, 1., "unfrosted sidebar fill must be opaque");
        assert_eq!(
            crate::appearance::window_background_appearance(false),
            gpui_kit::WindowBackgroundAppearance::Opaque,
            "unfrosted restores an opaque window"
        );

        assert_eq!(window.find("toggle-sidebar-frosted").checked(), Some(false), "switch should read unchecked while opaque");
        window.click("toggle-sidebar-frosted", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).sidebar_frosted, "toggle back restores frosted glass");
        assert!(crate::persist::load_settings().sidebar_frosted);
        let root_bg = Root::update(window, cx, |root, _, _| root.style().background.clone());
        assert_eq!(root_bg, Some(Fill::from(transparent_black())), "re-frosting restores the transparent Root fill");
    });
}

#[test]
fn sidebar_vibrancy_view_syncs_with_frosted_expanded_state() {
    // The real frosted glass is a native NSVisualEffectView (Sidebar
    // material) that window.rs installs behind the sidebar — gpui's own
    // Blurred background uses the faint Selection material. The view must
    // exist only while the sidebar is frosted AND mounted.
    assert!(crate::window::sidebar_vibrancy_active(true, false));
    assert!(!crate::window::sidebar_vibrancy_active(true, true), "collapsed unmounts the sidebar");
    assert!(!crate::window::sidebar_vibrancy_active(false, false), "unfrosted removes the view");
    assert!(!crate::window::sidebar_vibrancy_active(false, true));

    // Headless windows have no native handle — sync must no-op, not panic.
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, _cx| {
        crate::window::sync_sidebar_vibrancy(window, 255., true);
        crate::window::sync_sidebar_vibrancy(window, 255., false);
    });
}

/// Drive the contrast slider through `set_contrast` — the same path slider
/// events take — and return the resulting theme colors. `ThemeColor` has no
/// `PartialEq`, so callers compare `format!("{colors:?}")` snapshots.
fn set_contrast_pct(ws: &Entity<Workspace>, pct: f32, window: &mut Window, cx: &mut App) -> ThemeColor {
    ws.update(cx, |this, cx| this.set_contrast(&SliderEvent::Change(SliderValue::Single(pct)), window, cx));
    cx.global::<Theme>().colors
}

#[test]
fn contrast_scales_separation_from_stock() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let stock = format!("{:?}", cx.global::<Theme>().colors);

        // 100% reproduces the stock palette exactly.
        let at_100 = format!("{:?}", set_contrast_pct(&ws, 100., window, cx));
        assert_eq!(at_100, stock, "contrast 100 must equal the stock theme");

        // 150% widens the fg/bg lightness gap; 50% narrows it but stays
        // legible — the floor keeps text visible instead of collapsing it.
        let stock_delta = (cx.global::<Theme>().foreground.l - cx.global::<Theme>().background.l).abs();
        let at_150 = set_contrast_pct(&ws, 150., window, cx);
        let d150 = (at_150.foreground.l - at_150.background.l).abs();
        assert!(d150 > stock_delta, "contrast 150 should widen fg/bg separation ({d150} vs {stock_delta})");
        let at_50 = set_contrast_pct(&ws, 50., window, cx);
        let d50 = (at_50.foreground.l - at_50.background.l).abs();
        assert!(d50 < stock_delta, "contrast 50 should narrow fg/bg separation");
        assert!(d50 >= crate::appearance::MIN_LEGIBLE_DELTA - 0.01, "contrast 50 must stay legible ({d50})");

        // Drag history doesn't compound: after 50 → 150 → 100 the palette is
        // identical to stock, and re-applying 50 reproduces the same colors.
        let back_at_100 = format!("{:?}", set_contrast_pct(&ws, 100., window, cx));
        assert_eq!(back_at_100, stock, "returning to 100 must restore stock exactly");
        let at_50_again = format!("{:?}", set_contrast_pct(&ws, 50., window, cx));
        assert_eq!(at_50_again, format!("{at_50:?}"), "same percentage must give same colors regardless of history");
    });
}

#[test]
fn font_picker_search_filters_the_list() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_appearance(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Open the interface font dropdown — the search input takes focus.
        window.within("font-select").click("input", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // "zedmono" matches only ".ZedMono" in the fallback stack.
        window.input("zedmono", cx);
    });
    // The query input's Change event is delivered at the end of the update
    // cycle, so the filtered list + reset cursor land one update later.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Enter confirms the first match.
        window.press("enter", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_family, ".ZedMono", "search should narrow the list to the queried font");
    });
}
