//! Tests for `open_in`: command construction per editor, the recorded-spawn
//! fake, clipboard copy, persistence of the preferred editor, and headless
//! right-click menus on Changes/explorer rows. Narrow imports on purpose
//! (see `composer_testutil`).

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::git::{ChangeStatus, FileChange};
use crate::open_in::{FAIL_WITH, ISSUED, OpenCommand, PreferredEditor, open_command, reveal_command};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-openin-test-{}", std::process::id()));
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
    (ws.unwrap(), cx)
}

fn change(path: &str) -> FileChange {
    FileChange {
        path: path.into(),
        source: None,
        status: ChangeStatus::Modified,
        added: 1,
        deleted: 0,
        diff: None,
        staged: false,
        diff_load: 0,
    }
}

/// Wait for a background `run_open_command` task to record `n` commands.
fn until_issued(cx: &mut VisualTestContext, n: usize) {
    for _ in 0..200 {
        cx.run_until_parked();
        if ISSUED.lock().len() >= n {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("expected {n} issued commands, got {:?}", ISSUED.lock());
}

#[test]
fn reveal_command_targets_open_dash_r() {
    let cmd = reveal_command(std::path::Path::new("/repo/src/a.rs"));
    assert_eq!(cmd.program, "open");
    assert_eq!(cmd.args, ["-R", "/repo/src/a.rs"]);
}

#[test]
fn open_command_targets_each_editor_app() {
    let abs = std::path::Path::new("/repo/src/a.rs");
    for (editor, app) in [
        (PreferredEditor::VsCode, "Visual Studio Code"),
        (PreferredEditor::Cursor, "Cursor"),
        (PreferredEditor::Zed, "Zed"),
    ] {
        let cmd = open_command(editor, abs).unwrap_or_else(|| panic!("{editor:?} should build a command"));
        assert_eq!(cmd.program, "open");
        assert_eq!(cmd.args, ["-a", app, "/repo/src/a.rs"], "{editor:?}");
    }
    // Finder isn't an editor — "open in Finder" reveals the file.
    assert_eq!(open_command(PreferredEditor::Finder, abs), Some(reveal_command(abs)));
    // Ask never builds a command — the menu shows the picker instead.
    assert_eq!(open_command(PreferredEditor::Ask, abs), None);
}

#[test]
fn editor_names_and_labels_round_trip() {
    for editor in PreferredEditor::ALL {
        assert_eq!(PreferredEditor::from_name(editor.name()), editor, "name {}", editor.name());
        assert_eq!(PreferredEditor::from_label(editor.label()), editor, "label {}", editor.label());
    }
    assert_eq!(PreferredEditor::from_name(""), PreferredEditor::Ask, "empty persists as Ask");
    assert_eq!(PreferredEditor::from_name("emacs"), PreferredEditor::Ask, "unknown falls back to Ask");
}

#[test]
fn change_row_menu_reveals_and_copies() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("src/edited.rs")];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the file menu");
        window.within("popup-menu").click(0usize, cx); // Reveal in Finder
    });
    until_issued(cx, 1);
    let abs = ws.read_with(cx, |w, _| w.project.root().join("src/edited.rs").display().to_string());
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(std::path::Path::new(&abs))]);

    cx.update(|window, cx| {
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.within("popup-menu").click(2usize, cx); // Copy Path
    });
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    assert_eq!(clip, abs, "Copy Path should put the absolute path on the clipboard");
}

#[test]
fn open_in_editor_issues_open_dash_a() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.set_preferred_editor(PreferredEditor::Cursor, cx);
            this.changes = vec![change("src/edited.rs")];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.within("popup-menu").click(1usize, cx); // Open in Cursor
    });
    until_issued(cx, 1);
    let abs = ws.read_with(cx, |w, _| w.project.root().join("src/edited.rs").display().to_string());
    let expected = OpenCommand {
        program: "open".into(),
        args: vec!["-a".into(), "Cursor".into(), abs],
        action: "Open in Cursor".into(),
    };
    assert_eq!(ISSUED.lock().as_slice(), &[expected]);
}

#[test]
fn explorer_file_row_shows_file_actions() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project_files = vec!["README.md".into()];
            this.set_sidebar_tab(crate::views::sidebar::SidebarTab::Files, cx);
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("explorer-file", 0usize)).visible(), "file row renders");
        window.right_click(("explorer-file", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the file menu");
        window.within("popup-menu").click(0usize, cx); // Reveal in Finder
    });
    until_issued(cx, 1);
    let abs = ws.read_with(cx, |w, _| w.project.root().join("README.md").display().to_string());
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(std::path::Path::new(&abs))]);
}

#[test]
fn failed_open_surfaces_an_error_toast() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    *FAIL_WITH.lock() = Some("The application Visual Studio Code.app does not exist".into());
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.set_preferred_editor(PreferredEditor::VsCode, cx);
            this.open_in_editor("src/edited.rs", None, cx);
        });
    });
    until_issued(cx, 1);
    // The toast lands via update_in after the background run finishes.
    for _ in 0..200 {
        cx.run_until_parked();
        let toasts = cx.update(|window, cx| {
            let Some(Some(root)) = window.root::<Root>() else { return 0 };
            root.read(cx).notification.read(cx).notifications().len()
        });
        if toasts > 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("failure toast never landed");
}

#[test]
fn preferred_editor_persists() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.set_preferred_editor(PreferredEditor::Zed, cx));
    });
    assert_eq!(crate::persist::load_settings().preferred_editor, "zed");
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| this.set_preferred_editor(PreferredEditor::Ask, cx));
    });
    assert_eq!(crate::persist::load_settings().preferred_editor, "ask");
}
