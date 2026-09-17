//! Headless tests for the Cmd-=/Cmd--/Cmd-0 text-zoom shortcuts: they step
//! and reset the interface font size (the rem base), clamp at the shared
//! bounds, persist to settings.json and keep the Appearance stepper in sync.
//! Mount pattern matches `appearance_tests.rs`.

use gpui_kit::component::theme::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::appearance::{FONT_SIZE_DEFAULT, FONT_SIZE_MAX, FONT_SIZE_MIN};
use crate::composer_testutil::{open_workspace, type_and_send, use_sim};
use crate::workspace::Workspace;

/// Mount a `Workspace` with the real workspace keymap bound, so
/// `window.press` exercises the same path a physical keypress takes.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| cx.bind_keys(crate::workspace_keys()));
    open_workspace(cx)
}

#[test]
fn zoom_shortcuts_step_clamp_and_persist() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert_eq!(ws.read_with(cx, |ws, _| ws.font_size), FONT_SIZE_DEFAULT);

    // Cmd-= and Cmd-+ (Cmd-Shift-= on US layouts) both zoom in, half a px
    // per press.
    cx.update(|window, cx| {
        window.press("cmd-=", cx);
        window.press("cmd-shift-=->+", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_size, FONT_SIZE_DEFAULT + 1.);
        assert_eq!(f32::from(cx.global::<Theme>().font_size), FONT_SIZE_DEFAULT + 1.);
        assert_eq!(crate::persist::load_settings().font_size, FONT_SIZE_DEFAULT + 1.);
    });

    // Cmd-- steps back down.
    cx.update(|window, cx| window.press("cmd--", cx));
    cx.update(|_, cx| assert_eq!(ws.read(cx).font_size, FONT_SIZE_DEFAULT + 0.5));

    // Clamps at the max instead of wrapping or growing past it.
    cx.update(|_, cx| ws.update(cx, |ws, _| ws.font_size = FONT_SIZE_MAX));
    cx.update(|window, cx| window.press("cmd-=", cx));
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).font_size, FONT_SIZE_MAX);
        assert_eq!(crate::persist::load_settings().font_size, FONT_SIZE_MAX);
    });

    // And at the min.
    cx.update(|_, cx| ws.update(cx, |ws, _| ws.font_size = FONT_SIZE_MIN));
    cx.update(|window, cx| window.press("cmd--", cx));
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).font_size, FONT_SIZE_MIN);
        assert_eq!(crate::persist::load_settings().font_size, FONT_SIZE_MIN);
    });
}

#[test]
fn zoom_actions_dispatch_and_reset_restores_default() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| ws.update(cx, |ws, _| ws.font_size = 18.));
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::ZoomIn), cx));
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(ws.read(cx).font_size, 18.5, "ZoomIn should grow font_size by the half-px step"));
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::ZoomReset), cx));
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(ws.read(cx).font_size, FONT_SIZE_DEFAULT, "ZoomReset should restore the default");
        assert_eq!(crate::persist::load_settings().font_size, FONT_SIZE_DEFAULT);
    });
}

#[test]
fn appearance_stepper_reflects_zoom() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Open Settings → Appearance; the stepper starts at the default.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-appearance", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("font-select-size").label(), Some("14px"));
    });
    // Zooming with the settings open updates the stepper's readout.
    cx.update(|window, cx| window.press("cmd-=", cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_size, 14.5);
        assert_eq!(window.find("font-select-size").label(), Some("14.5px"), "the stepper should track zoom");
    });
    // The stepper and the shortcut share one write path — clicking + lands
    // on the same field.
    cx.update(|window, cx| window.click("font-select-inc", cx));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).font_size, 15.);
        assert_eq!(window.find("font-select-size").label(), Some("15px"));
    });
}

#[test]
fn zoom_scales_rendered_message_text() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    use_sim(&ws, cx);
    type_and_send(cx, "hello");
    // The message body carries .text_size(px(font_size)) — its rendered
    // height must track the setting, not just the theme global.
    let before = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        f32::from(window.find(("md-body", 0usize)).bounds().size.height)
    });
    cx.update(|window, cx| window.press("cmd-=", cx));
    let after = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        f32::from(window.find(("md-body", 0usize)).bounds().size.height)
    });
    assert!(after > before, "md-body height {after} should exceed {before} after Cmd-=");
}
