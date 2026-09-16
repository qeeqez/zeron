//! Tests for workspace trust: the persisted trusted-folders list, the
//! restricted mode an untrusted folder opens in, and the first-open dialog.
//! `HOME` is sandboxed per test so the trusted list never touches the real
//! profile. Same harness as `ui_tests.rs` — plain `#[test]` + narrow imports.

use std::path::PathBuf;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::AccessMode;
use crate::workspace::Workspace;

/// Redirect `~` into a throwaway dir; nextest runs each test in its own
/// process, so no other thread can observe HOME mid-write.
fn sandbox_home(name: &str) {
    let dir = std::env::temp_dir().join(format!("rixlcode-trust-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
}

/// A fresh project folder under the system temp dir (canonicalized, like
/// `Project::open` leaves it).
fn temp_project(leaf: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-trust-proj-{leaf}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Mount a `Workspace` bound to `project` — the test twin of
/// `lifecycle::open_workspace_window_for` minus the trust gate, so tests
/// drive `restrict_untrusted`/`trust_project` directly.
fn mount<'a>(cx: &'a mut TestAppContext, project: &crate::project::Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::for_project(project.clone(), window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// The `Workspace` entity behind a window's `Root` view.
fn workspace_of(app: &TestAppContext, handle: AnyWindowHandle) -> Entity<Workspace> {
    app.update(|cx| {
        handle
            .read::<Root, _, _>(cx, |root, cx| root.read(cx).view().clone().downcast::<Workspace>().ok())
            .ok()
            .flatten()
            .expect("window should host a workspace")
    })
}

/// Open `dir` through the real `lifecycle::open_project` path and return the
/// new window's handle.
fn open_project_window(app: &mut TestAppContext, dir: &std::path::Path) -> AnyWindowHandle {
    let before = app.windows().len();
    app.update(|cx| crate::lifecycle::open_project(dir, cx));
    app.run_until_parked();
    assert_eq!(app.windows().len(), before + 1, "open_project should create a window");
    *app.windows().last().unwrap()
}

#[test]
fn untrusted_folder_opens_restricted() {
    let mut app = TestAppContext::single();
    sandbox_home("restricted");
    let project = crate::project::Project::open(temp_project("untrusted"));
    let (ws, cx) = mount(&mut app, &project);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.restrict_untrusted(cx)));
    ws.read_with(cx, |this, _| {
        assert!(!this.trusted);
        assert_eq!(this.access, AccessMode::Supervised, "untrusted forces read-only/ask");
        assert!(this.chats.iter().all(|c| c.access == Some(AccessMode::Supervised)), "thread stamps are clamped too");
        assert_eq!(this.turn_context().access, AccessMode::Supervised, "turns run restricted");
    });
}

#[test]
fn trust_persists_and_restores_access() {
    let mut app = TestAppContext::single();
    sandbox_home("trust");
    // A configured permissive mode — trusting must restore it, not the clamp.
    let mut s = crate::persist::load_settings();
    s.access = AccessMode::FullAccess.name().to_string();
    crate::persist::save_settings(&s);
    let project = crate::project::Project::open(temp_project("trustme"));
    let (ws, cx) = mount(&mut app, &project);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.restrict_untrusted(cx)));
    cx.update(|_, cx| ws.update(cx, |this, cx| this.trust_project(cx)));
    ws.read_with(cx, |this, _| {
        assert!(this.trusted);
        assert_eq!(this.access, AccessMode::FullAccess, "trust restores the configured mode");
        assert_eq!(this.turn_context().access, AccessMode::FullAccess);
    });
    assert!(crate::trust::is_trusted(project.root()));
    let stored = crate::persist::load_settings().trusted_folders;
    assert!(stored.contains(&project.root().to_string_lossy().into_owned()), "trusted list persisted: {stored:?}");
}

#[test]
fn dont_trust_keeps_read_only() {
    let mut app = TestAppContext::single();
    sandbox_home("dont-trust");
    let project = crate::project::Project::open(temp_project("wary"));
    let (ws, cx) = mount(&mut app, &project);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.restrict_untrusted(cx)));
    // Dismissing the dialog changes nothing — and the access picker can't
    // lift the restriction while the folder stays untrusted.
    cx.update(|_, cx| ws.update(cx, |this, cx| this.set_access(AccessMode::FullAccess, cx)));
    ws.read_with(cx, |this, _| {
        assert!(!this.trusted);
        assert_eq!(this.access, AccessMode::Supervised, "set_access can't escape restricted mode");
        assert_eq!(this.turn_context().access, AccessMode::Supervised);
    });
    assert!(!crate::trust::is_trusted(project.root()), "nothing was persisted");
}

#[test]
fn trusted_list_persists_and_canonicalizes() {
    sandbox_home("canon");
    let dir = temp_project("canon");
    // A non-canonical spelling of the same folder — `dir/../leaf`.
    let noncanon = dir.join("..").join(dir.file_name().unwrap());
    crate::trust::trust(&noncanon);
    let stored = crate::persist::load_settings().trusted_folders;
    assert_eq!(stored, vec![dir.to_string_lossy().into_owned()], "stored canonicalized: {stored:?}");
    assert!(crate::trust::is_trusted(&noncanon));
    assert!(crate::trust::is_trusted(&dir));
    // Re-trusting doesn't duplicate the entry.
    crate::trust::trust(&dir);
    assert_eq!(crate::persist::load_settings().trusted_folders.len(), 1);
}

#[test]
fn untrusted_open_shows_trust_dialog() {
    let mut app = TestAppContext::single();
    sandbox_home("dialog");
    app.update(gpui_kit::init);
    let dir = temp_project("prompt");
    let handle = open_project_window(&mut app, &dir);
    let ws = workspace_of(&app, handle);
    let mut cx = VisualTestContext::from_window(handle, &app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("trust-dialog").visible(), "untrusted folder should prompt on open");
    });
    ws.read_with(&app, |this, _| {
        assert!(!this.trusted);
        assert_eq!(this.access, AccessMode::Supervised);
    });
    // "Trust" persists the folder, lifts the restriction and closes.
    cx.update(|window, cx| {
        window.click("trust-folder", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("trust-dialog").is_none(), "dialog should close on Trust");
        assert!(window.try_find("trust-banner").is_none(), "banner clears once trusted");
    });
    ws.read_with(&app, |this, _| assert!(this.trusted));
    assert!(crate::trust::is_trusted(&dir));
}

#[test]
fn dont_trust_button_keeps_restricted_with_banner() {
    let mut app = TestAppContext::single();
    sandbox_home("decline");
    app.update(gpui_kit::init);
    let dir = temp_project("decline");
    let handle = open_project_window(&mut app, &dir);
    let ws = workspace_of(&app, handle);
    let mut cx = VisualTestContext::from_window(handle, &app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("dont-trust-folder", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("trust-dialog").is_none(), "dialog should close");
        // The banner stays as the persistent notice + trust-later path.
        assert!(window.find("trust-banner").visible(), "restricted banner should remain");
    });
    ws.read_with(&app, |this, _| {
        assert!(!this.trusted);
        assert_eq!(this.access, AccessMode::Supervised);
    });
    assert!(!crate::trust::is_trusted(&dir));
}

#[test]
fn trusted_folder_reopens_without_dialog() {
    let mut app = TestAppContext::single();
    sandbox_home("reopen");
    app.update(gpui_kit::init);
    let dir = temp_project("reopen");
    crate::trust::trust(&dir);
    let handle = open_project_window(&mut app, &dir);
    let ws = workspace_of(&app, handle);
    let mut cx = VisualTestContext::from_window(handle, &app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("trust-dialog").is_none(), "trusted folder should not prompt");
        assert!(window.try_find("trust-banner").is_none(), "no restricted banner when trusted");
    });
    ws.read_with(&app, |this, _| {
        assert!(this.trusted);
        assert_eq!(this.access, AccessMode::Auto, "trusted folder keeps the configured default");
    });
}
