//! Tests for update checking: semver ordering, the `apply_release` state
//! transitions (including the skipped-version gate and last-check
//! persistence), and headless runs of the startup check and the "Check for
//! Updates" action — all against a fake `ReleaseSource`, never the network.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use std::cmp::Ordering;
use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, VisualTestContext};

use crate::update::{self, Release, ReleaseOutcome, ReleaseSource, UpdateState, UpdateStatus};
use crate::workspace::Workspace;

/// A release source returning a fixed result.
struct FakeReleases(Result<Release, String>);

impl ReleaseSource for FakeReleases {
    fn latest(&self) -> Result<Release, String> {
        self.0.clone()
    }
}

fn release(tag: &str) -> Release {
    Release {
        tag: tag.to_string(),
        url: format!("https://github.com/rixlhq/code/releases/tag/{tag}"),
    }
}

fn fake(tag: &str) -> Arc<dyn ReleaseSource> {
    Arc::new(FakeReleases(Ok(release(tag))))
}

/// Redirect `~` into a throwaway dir; nextest runs each test in its own
/// process, so no other thread can observe HOME mid-write.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-update-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
}

/// Mount a `Workspace` in a headless window — the startup update check runs
/// as part of `Workspace::new`.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, AnyWindowHandle, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The test platform drops system notifications posted without an
        // identity, matching the Linux/Windows behavior main.rs sets up.
        cx.set_app_identity("com.rixl.rixlcode", "Rixl Code");
    });
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

/// Pump the executors until `cond` holds — the fetch lands on the
/// background executor, so a single `run_until_parked` may not suffice.
fn until(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if workspace.read_with(cx, |ws, _| cond(ws)) {
            return;
        }
    }
    panic!("condition never held");
}

/// In-app toasts mounted under the Root's notification layer.
fn toast_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| {
        let Some(Some(root)) = window.root::<Root>() else { return 0 };
        root.read(cx).notification.read(cx).notifications().len()
    })
}

#[test]
fn semver_orders_release_tags() {
    assert_eq!(update::semver_compare("v0.2.0", "0.1.0"), Some(Ordering::Greater));
    assert_eq!(update::semver_compare("0.1.0", "0.1.0"), Some(Ordering::Equal));
    assert_eq!(update::semver_compare("0.1.0", "v0.2.0"), Some(Ordering::Less));
    // Missing components default to zero; build metadata is ignored.
    assert_eq!(update::semver_compare("v1.0", "1.0.0"), Some(Ordering::Equal));
    assert_eq!(update::semver_compare("1.0.0+build.5", "1.0.0"), Some(Ordering::Equal));
}

#[test]
fn semver_orders_prereleases() {
    // A prerelease sorts before its release.
    assert_eq!(update::semver_compare("v1.0.0-rc.1", "1.0.0"), Some(Ordering::Less));
    assert_eq!(update::semver_compare("1.0.0", "v1.0.0-rc.1"), Some(Ordering::Greater));
    // Numeric identifiers order numerically and below alphanumeric ones.
    assert_eq!(update::semver_compare("1.0.0-alpha.2", "1.0.0-alpha.10"), Some(Ordering::Less));
    assert_eq!(update::semver_compare("1.0.0-alpha.1", "1.0.0-alpha.beta"), Some(Ordering::Less));
    // Fewer identifiers win when the shared prefix is equal.
    assert_eq!(update::semver_compare("1.0.0-alpha", "1.0.0-alpha.1"), Some(Ordering::Less));
    // Garbage never counts as newer — including a leading-zero numeric
    // identifier, which semver rejects outright.
    assert_eq!(update::semver_compare("not-a-version", "0.1.0"), None);
    assert_eq!(update::semver_compare("0.1.0-01", "0.1.0"), None);
}

#[test]
fn newer_release_marks_update_available() {
    sandbox_home();
    let mut state = UpdateState::default();
    let outcome = update::apply_release(&mut state, &release("v99.0.0"));
    assert_eq!(outcome, ReleaseOutcome::Available);
    assert_eq!(state.status, UpdateStatus::Available("v99.0.0".to_string()));
    assert!(!state.skipped);
    let s = crate::persist::load_settings();
    assert_eq!(s.update_latest, "v99.0.0", "the pending tag should persist");
    assert!(s.update_last_check.is_some(), "the check time should persist");
}

#[test]
fn same_or_older_release_is_up_to_date() {
    sandbox_home();
    for tag in [env!("CARGO_PKG_VERSION"), "v0.0.1"] {
        let mut state = UpdateState::default();
        let outcome = update::apply_release(&mut state, &release(tag));
        assert_eq!(outcome, ReleaseOutcome::UpToDate, "{tag} should not count as newer");
        assert_eq!(state.status, UpdateStatus::UpToDate);
        assert!(crate::persist::load_settings().update_latest.is_empty(), "no pending tag should persist");
    }
}

#[test]
fn skipped_release_does_not_renotify() {
    sandbox_home();
    let mut state = UpdateState::default();
    assert_eq!(update::apply_release(&mut state, &release("v99.0.0")), ReleaseOutcome::Available);
    // Dismissing the update records the skip and clears the pending tag.
    state.skip("v99.0.0");
    assert!(state.skipped);
    assert_eq!(crate::persist::load_settings().update_skip, "v99.0.0");
    // The same tag landing again stays quiet; a newer one still surfaces.
    assert_eq!(update::apply_release(&mut state, &release("v99.0.0")), ReleaseOutcome::Skipped);
    assert!(state.skipped);
    assert_eq!(update::apply_release(&mut state, &release("v99.1.0")), ReleaseOutcome::Available);
    assert!(!state.skipped);
}

#[test]
fn pending_update_restores_from_settings() {
    sandbox_home();
    crate::persist::save_settings(&crate::persist::Settings { update_latest: "v9.9.9".to_string(), ..Default::default() });
    let state = UpdateState::restored(&crate::persist::load_settings());
    assert_eq!(state.status, UpdateStatus::Available("v9.9.9".to_string()));
    assert!(!state.skipped);
}

#[test]
fn startup_check_finds_update_and_notifies() {
    let mut app = TestAppContext::single();
    sandbox_home();
    update::set_test_source(fake("v99.0.0"));
    let (workspace, _handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| matches!(ws.update.status, UpdateStatus::Available(_)));
    let (status, url) = workspace.read_with(cx, |ws, _| (ws.update.status.clone(), ws.update.url.clone()));
    assert_eq!(status, UpdateStatus::Available("v99.0.0".to_string()));
    assert!(url.contains("v99.0.0"), "the release page URL should persist for Download");
    assert_eq!(toast_count(cx), 1, "a new release should post one toast");
    let notes = cx.delivered_system_notifications();
    assert_eq!(notes.len(), 1, "the update should also reach the OS notification center");
    assert!(notes[0].body.contains("v99.0.0"), "the notification names the release");
}

#[test]
fn check_for_updates_action_reports_up_to_date() {
    let mut app = TestAppContext::single();
    sandbox_home();
    app.update(crate::install_app_actions);
    update::set_test_source(fake("v0.0.1"));
    let (workspace, handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| ws.update.status == UpdateStatus::UpToDate);
    // The action targets the active window — activate it like a real menu
    // click would.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    TestAppContext::dispatch_action(cx, handle, crate::CheckForUpdates);
    for _ in 0..64 {
        cx.run_until_parked();
        if toast_count(cx) > 0 {
            break;
        }
    }
    assert_eq!(toast_count(cx), 1, "a manual check should report the outcome");
    let status = workspace.read_with(cx, |ws, _| ws.update.status.clone());
    assert_eq!(status, UpdateStatus::UpToDate);
}

#[test]
fn check_for_updates_action_reports_available() {
    let mut app = TestAppContext::single();
    sandbox_home();
    app.update(crate::install_app_actions);
    update::set_test_source(fake("v99.0.0"));
    let (workspace, handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| matches!(ws.update.status, UpdateStatus::Available(_)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    TestAppContext::dispatch_action(cx, handle, crate::CheckForUpdates);
    until(&workspace, cx, |ws| matches!(ws.update.status, UpdateStatus::Available(_)));
    assert_eq!(toast_count(cx), 1, "the same tag replaces the toast rather than stacking");
}

#[test]
fn profile_about_row_shows_pending_update() {
    let mut app = TestAppContext::single();
    sandbox_home();
    update::set_test_source(fake("v99.0.0"));
    let (workspace, _handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| matches!(ws.update.status, UpdateStatus::Available(_)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-profile", cx);
        window.draw(cx).clear(cx);
        let row = window.find("profile-update");
        assert_eq!(row.label(), Some("v99.0.0 available"), "the About row names the pending release");
        assert!(window.find("profile-update-download").visible(), "a Download button should show");
        window.click("profile-update-download", cx);
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/rixlhq/code/releases/tag/v99.0.0"));
}
