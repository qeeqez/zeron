//! Tests for the "Check for Updates" result dialog: up-to-date, a newer
//! release (View Release opens the page and closes), a repo with no
//! releases, and a fetch failure — all against a fake `ReleaseSource`, never
//! the network. Same mount harness as `update_tests.rs` — duplicated because
//! sibling test files can't share private helpers.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, VisualTestContext};

use crate::update::{self, Release, ReleaseSource, UpdateStatus};
use crate::workspace::Workspace;

/// A release source returning a fixed result.
struct FakeReleases(Result<Option<Release>, String>);

impl ReleaseSource for FakeReleases {
    fn latest(&self) -> Result<Option<Release>, String> {
        self.0.clone()
    }
}

fn release(tag: &str) -> Release {
    Release {
        tag: tag.to_string(),
        url: format!("https://github.com/rixlhq/code/releases/tag/{tag}"),
        name: Some(format!("Rixl Code {tag}")),
        notes: Some("Added\n- something new".to_string()),
    }
}

fn fake(tag: &str) -> Arc<dyn ReleaseSource> {
    Arc::new(FakeReleases(Ok(Some(release(tag)))))
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

/// True while the update result dialog is mounted — draws a frame first so
/// the element registry reflects the latest state.
fn dialog_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.try_find("update-dialog").is_some_and(|d| d.visible())
    })
}

/// The dialog's aria label — the copy the test asserts on.
fn dialog_label(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, _| window.find("update-dialog").label().map(str::to_string))
}

/// Dispatch `CheckForUpdates` on the active window like a menu click, then
/// pump until the result dialog mounts.
fn check_via_menu(handle: AnyWindowHandle, cx: &mut VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    TestAppContext::dispatch_action(cx, handle, crate::CheckForUpdates);
    for _ in 0..64 {
        cx.run_until_parked();
        if dialog_open(cx) {
            return;
        }
    }
    panic!("the update dialog never opened");
}

#[test]
fn check_for_updates_action_reports_up_to_date() {
    let mut app = TestAppContext::single();
    sandbox_home();
    app.update(crate::install_app_actions);
    update::set_test_source(fake("v0.0.1"));
    let (workspace, handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| ws.update.status == UpdateStatus::UpToDate);
    check_via_menu(handle, cx);
    assert_eq!(dialog_label(cx).as_deref(), Some("You're on the latest version (v0.1.0). No newer release is available."));
    assert_eq!(toast_count(cx), 0, "the dialog reports; no toast on top");
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
    check_via_menu(handle, cx);
    assert_eq!(dialog_label(cx).as_deref(), Some("v99.0.0 available"));
    // View Release opens the release page and dismisses the dialog. The
    // dialog slides in — let the animation finish so the click lands on the
    // button's final position, not its animated one.
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| window.click("update-dialog-view-release", cx));
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/rixlhq/code/releases/tag/v99.0.0"));
    cx.run_until_parked();
    assert!(!dialog_open(cx), "View Release should close the dialog");
}

#[test]
fn check_for_updates_action_reports_no_releases() {
    let mut app = TestAppContext::single();
    sandbox_home();
    app.update(crate::install_app_actions);
    // The default `NoReleases` source answers Ok(None) — a repo whose
    // `latest` endpoint 404s.
    let (workspace, handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| ws.update.status == UpdateStatus::UpToDate);
    check_via_menu(handle, cx);
    assert_eq!(dialog_label(cx).as_deref(), Some("You're on the latest version (v0.1.0). The repository has no published releases yet."));
}

#[test]
fn check_for_updates_action_reports_failure() {
    let mut app = TestAppContext::single();
    sandbox_home();
    app.update(crate::install_app_actions);
    update::set_test_source(Arc::new(FakeReleases(Err("offline".to_string()))));
    let (workspace, handle, cx) = mount(&mut app);
    until(&workspace, cx, |ws| ws.update.status == UpdateStatus::Unknown);
    check_via_menu(handle, cx);
    assert_eq!(dialog_label(cx).as_deref(), Some("Couldn't check for updates. offline"));
    assert_eq!(toast_count(cx), 0, "the dialog reports; no toast on top");
}
