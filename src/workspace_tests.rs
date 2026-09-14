//! Headless tests for workspace lifecycle and theme persistence: the window
//! must release its `Workspace` on close (no entity cycles), palette theme
//! commands must persist like the Appearance cards do, and an unrelated
//! `save_settings` must not clobber a theme another window wrote. Same
//! harness as `ui_tests.rs` — plain `#[test]` + narrow imports.

use gpui_kit::component::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::component::theme::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-workspace-test-{}", std::process::id()));
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

/// Regression: `Workspace` → `SettingsPanel` → subscription closures used to
/// hold strong `Entity<Workspace>` handles, cycling the window's whole entity
/// graph — closing the window leaked the workspace and its chats (running
/// subprocesses included, since `Chat::drop` never ran).
#[test]
fn closing_window_releases_workspace() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Open settings once so the panel's inputs/subscriptions exist.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_settings(window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("settings-screen").visible());
    });
    let weak = ws.downgrade();
    drop(ws);
    cx.update(|window, _cx| window.remove_window());
    app.run_until_parked();
    assert!(weak.upgrade().is_none(), "workspace should release when its window closes");
}

/// The palette's "Switch to Dark Theme" dispatches `ThemeDark` — it must go
/// through the same persist path as the Appearance theme cards, not just
/// `Theme::change`.
#[test]
fn palette_theme_command_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.open_palette(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.palette.update(cx, |state, cx| state.set_query("dark", window, cx));
        });
        window.draw(cx).clear(cx);
        // Only "Switch to Dark Theme" matches — confirming runs it.
        assert_eq!(ws.read(cx).palette.read(cx).matched_count(), 1);
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    // Confirm → ThemeDark dispatch → set_theme are all deferred.
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).theme, "dark", "palette command should update the workspace theme");
        assert!(cx.global::<Theme>().mode.is_dark(), "dark theme should apply");
        assert_eq!(crate::persist::load_settings().theme, "dark", "palette theme choice should persist");
    });
}

/// Multi-window: window B snapshots `settings.theme` at construction. When
/// window A changes the theme, B's next unrelated save must not write its
/// stale value back — and B adopts the persisted choice.
#[test]
fn unrelated_save_preserves_persisted_theme() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Another window's theme change lands on disk after this workspace
    // loaded its snapshot.
    let mut settings = crate::persist::load_settings();
    settings.theme = "dark".into();
    crate::persist::save_settings(&settings);

    cx.update(|_window, cx| {
        // An unrelated save (sidebar toggle) must preserve the file's theme.
        ws.update(cx, |this, cx| this.toggle_sidebar(cx));
    });
    cx.update(|_window, cx| {
        assert_eq!(crate::persist::load_settings().theme, "dark", "unrelated save must not clobber the theme");
        assert_eq!(ws.read(cx).theme, "dark", "the stale workspace should adopt the persisted theme");
    });

    // This window's own theme change still persists.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.set_theme("light", window, cx));
    });
    cx.update(|_window, _cx| {
        assert_eq!(crate::persist::load_settings().theme, "light", "own theme change should persist");
    });
}
