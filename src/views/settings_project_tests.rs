//! Headless tests for the Project settings section: the setup-script field
//! renders seeded from the project's `state.json`, typing marks it dirty,
//! and Save persists `ProjectState.setup_script`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::project::{Project, ProjectState};
use crate::workspace::Workspace;

/// A fresh temp dir (HOME and project roots both live under it).
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-proj-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mount a `Workspace` bound to `project` — HOME must already point at the
/// test's temp dir.
fn mount_at(cx: &mut TestAppContext, project: Project) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::for_project(project, window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// `mount_at` with `HOME` redirected to a fresh temp dir first.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", temp_dir(name)) };
    mount_at(cx, project)
}

/// Open settings and switch to the Project section.
fn open_project_section(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.open_settings(window, cx));
    window.draw(cx).clear(cx);
    window.click("settings-nav-project", cx);
    window.draw(cx).clear(cx);
}

#[test]
fn setup_script_field_edits_and_saves() {
    let root = temp_dir("edit-proj");
    let project = Project::open(&root);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "edit", project.clone());
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        assert!(window.find("settings-section-project").visible(), "section should render");
        assert!(window.find("setup-script-field").visible(), "textarea should render");
        assert_eq!(window.find("setup-script-status").label(), Some("Saved"), "fresh field starts clean");

        // Type into the field — the status flips to unsaved.
        ws.update(cx, |this, cx| this.setup_script_input.update(cx, |s, cx| s.focus(window, cx)));
        window.input("npm install", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("setup-script-status").label(), Some("Unsaved changes"));

        // Save persists to the project's state.json and flips back.
        window.click("setup-script-save", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("setup-script-status").label(), Some("Saved"));
    });
    assert_eq!(ws.read_with(cx, |w, _| w.setup_script.clone()), "npm install");
    assert_eq!(project.load_state().setup_script, "npm install", "the script persists to state.json");
}

#[test]
fn setup_script_field_seeds_from_project_state() {
    let root = temp_dir("seed-proj");
    let project = Project::open(&root);
    project.save_state(&ProjectState { setup_script: "make setup".into(), ..Default::default() });

    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "seed", project);
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        assert_eq!(window.find("setup-script-status").label(), Some("Saved"), "seeded field starts clean");
    });
    let value = ws.read_with(cx, |w, app| w.setup_script_input.read(app).value().to_string());
    assert_eq!(value, "make setup");
}
