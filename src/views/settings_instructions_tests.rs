//! Headless tests for the Custom Instructions settings section: the field
//! renders seeded from settings, typing marks it dirty, and Save persists
//! to `Settings.instructions` — plus the project-file note.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::workspace::Workspace;

/// A fresh temp dir (HOME and project roots both live under it).
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-instr-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mount a `Workspace` bound to `project` — HOME must already point at the
/// test's temp dir.
fn mount_at(cx: &mut TestAppContext, project: crate::project::Project) -> (Entity<Workspace>, &mut VisualTestContext) {
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
fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: crate::project::Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", temp_dir(name)) };
    mount_at(cx, project)
}

/// Open settings and switch to the Custom Instructions section.
fn open_instructions(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.open_settings(window, cx));
    window.draw(cx).clear(cx);
    window.click("settings-nav-instructions", cx);
    window.draw(cx).clear(cx);
}

#[test]
fn instructions_section_edits_and_saves() {
    let root = temp_dir("edit-proj");
    std::fs::write(root.join("AGENTS.md"), "project rules").unwrap();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "edit", crate::project::Project::open(&root));
    cx.update(|window, cx| {
        open_instructions(&ws, window, cx);
        assert!(window.find("settings-section-instructions").visible(), "section should render");
        assert!(window.find("instructions-field").visible(), "textarea should render");
        assert_eq!(window.find("instructions-status").label(), Some("Saved"), "fresh field starts clean");
        // The project note names the discovered file.
        let note = window.find("instructions-project-note");
        assert!(note.visible());
        assert_eq!(note.label(), Some("AGENTS.md is appended after the global instructions"));

        // Type into the field — the status flips to unsaved.
        ws.update(cx, |this, cx| this.instructions_input.update(cx, |s, cx| s.focus(window, cx)));
        window.input("be terse", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("instructions-status").label(), Some("Unsaved changes"));

        // Save persists to settings.json and flips the status back.
        window.click("instructions-save", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("instructions-status").label(), Some("Saved"));
    });
    assert_eq!(ws.read_with(cx, |w, _| w.instructions.clone()), "be terse");
    assert_eq!(crate::persist::load_settings().instructions, "be terse");
}

#[test]
fn instructions_field_seeds_from_settings() {
    let root = temp_dir("seed-proj");
    let home = temp_dir("seed-home");
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("HOME", &home) };
    let mut s = crate::persist::load_settings();
    s.instructions = "seeded rules".to_string();
    crate::persist::save_settings(&s);

    let mut app = TestAppContext::single();
    let (ws, cx) = mount_at(&mut app, crate::project::Project::open(&root));
    cx.update(|window, cx| {
        open_instructions(&ws, window, cx);
        let note = window.find("instructions-project-note");
        assert_eq!(note.label(), Some("No project instructions file — add one of: AGENTS.md, CLAUDE.md, .rixl/instructions.md"));
    });
    let value = ws.read_with(cx, |w, app| w.instructions_input.read(app).value().to_string());
    assert_eq!(value, "seeded rules");
}
