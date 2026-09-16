//! Headless tests for the app icon: the embedded asset loads through
//! `AppAssets` (and still delegates to the gpui-kit icon catalog), and both
//! About surfaces — the dialog and the Profile settings block — render it.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, AssetSource, Entity, TestAppContext, VisualTestContext};

use crate::app_icon::{APP_ICON_PATH, AppAssets};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, AnyWindowHandle, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-icon-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
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

#[test]
fn app_icon_asset_loads() {
    let assets = AppAssets::new();
    let bytes = assets.load(APP_ICON_PATH).unwrap().expect("the app icon should be embedded");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "the app icon asset should be a PNG");
    // Delegation: the gpui-kit icon catalog still resolves through AppAssets.
    let icon = gpui_kit::assets::IconName::Check.path();
    assert!(assets.load(&icon).unwrap().is_some(), "lucide icons should still load");
    assert!(assets.list("icons/").unwrap().iter().any(|p| p.as_ref() == APP_ICON_PATH), "the app icon should list under icons/");
}

#[test]
fn about_dialog_shows_app_icon() {
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
        assert!(window.find("about-app-icon").visible(), "About should render the app icon");
    });
}

#[test]
fn profile_about_block_shows_app_icon() {
    let mut app = TestAppContext::single();
    let (_ws, _handle, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-profile", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-profile").visible(), "profile section should show");
        assert!(window.find("profile-app-icon").visible(), "the About block should render the app icon");
    });
}
