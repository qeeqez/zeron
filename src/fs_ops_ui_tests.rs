//! Headless explorer tests for `fs_ops`: the row/header context menus, the
//! inline name input (Enter commits, Escape cancels, an invalid name stays
//! armed), and the delete confirm — with `run` faked so the workspace's
//! real project root is never touched. Narrow imports on purpose (see
//! `composer_testutil`); helpers duplicate `fs_ops_tests` since sibling
//! test files can't share private fns.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, SharedString, TestAppContext, VisualTestContext, Window};

use crate::files::fs_ops::{FAIL_WITH, FsOp, ISSUED};
use crate::views::sidebar::SidebarTab;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile. The launch
/// file scan is awaited so it can't overwrite a test's canned list.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-fsops-test-{}", std::process::id()));
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
    let ws = ws.unwrap();
    // The launch scan runs on the background executor — poll until it lands
    // instead of assuming run_until_parked covers real threads.
    for _ in 0..200 {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| !ws.project_files.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (ws, cx)
}

/// Wait for a background `run_fs_op` task to record `n` ops.
fn until_issued(cx: &mut VisualTestContext, n: usize) {
    for _ in 0..200 {
        cx.run_until_parked();
        if ISSUED.lock().len() >= n {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("expected {n} issued ops, got {:?}", ISSUED.lock());
}

/// Switch to the Files tab with a canned file list loaded — `src` expanded
/// so its children render regardless of what the launch scan seeded.
fn open_explorer(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.project_files = vec![SharedString::from("src/lib.rs"), SharedString::from("README.md")];
        this.explorer.expanded.insert("src".to_string());
        this.set_sidebar_tab(SidebarTab::Files, cx);
    });
    window.draw(cx).clear(cx);
}

/// Click the popup-menu item with `label` — panics when it isn't offered.
fn click_menu_item(window: &mut Window, label: &str, cx: &mut App) {
    let item = snapshots(window)
        .iter()
        .find(|s| s.label() == Some(label))
        .unwrap_or_else(|| panic!("menu should offer {label}"))
        .clone();
    let id = item.path().last().unwrap().clone();
    window.within("popup-menu").click(id, cx);
}

fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window).iter().filter_map(|s| s.label().map(|l| l.to_string())).collect()
}

#[test]
fn dir_menu_offers_create_rename_delete() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        window.right_click(("explorer-dir", 0usize), cx); // src
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        for label in ["New File…", "New Folder…", "Rename…", "Delete"] {
            assert!(labels.iter().any(|l| l == label), "dir menu offers {label}: {labels:?}");
        }
        window.press("escape", cx);
        window.draw(cx).clear(cx);
    });
}

#[test]
fn new_file_dispatches_create_op() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        window.right_click(("explorer-dir", 0usize), cx); // src
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "New File…", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("explorer-input").visible(), "inline input mounts inside src");
    });
    // The deferred focus lands between updates; typing then goes to the input.
    cx.update(|window, cx| {
        window.input("new.rs", cx);
        window.press("enter", cx);
    });
    until_issued(cx, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[FsOp::NewFile("src/new.rs".into())]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).explorer.editing.is_none(), "edit disarmed after commit");
        assert!(window.try_find("explorer-input").is_none(), "input unmounts");
    });
}

#[test]
fn header_menu_creates_at_the_root() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        window.right_click("explorer-header", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "New Folder…"), "header menu offers New Folder…: {labels:?}");
        assert!(!labels.iter().any(|l| l == "Delete"), "header menu has no Delete: {labels:?}");
        click_menu_item(window, "New Folder…", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.input("docs", cx);
        window.press("enter", cx);
    });
    until_issued(cx, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[FsOp::NewFolder("docs".into())]);
}

#[test]
fn rename_dispatches_rename_op() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        window.right_click(("explorer-file", 1usize), cx); // src/lib.rs
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Rename…", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("explorer-input").visible(), "rename input replaces the row");
    });
    // Seeded with the current name, fully selected — typing replaces it.
    cx.update(|window, cx| {
        window.input("renamed.rs", cx);
        window.press("enter", cx);
    });
    until_issued(cx, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[FsOp::Rename { old: "src/lib.rs".into(), new: "src/renamed.rs".into() }]);
}

#[test]
fn delete_confirms_then_dispatches() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        window.right_click(("explorer-file", 1usize), cx); // src/lib.rs
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Delete", cx);
    });
    // Cancelling the confirm dispatches nothing.
    assert!(cx.has_pending_prompt(), "delete asks for confirmation");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(ISSUED.lock().is_empty(), "cancelled delete dispatches nothing");
    // Confirming dispatches the delete op.
    cx.update(|window, cx| {
        window.right_click(("explorer-file", 1usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Delete", cx);
    });
    cx.simulate_prompt_answer("Delete");
    until_issued(cx, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[FsOp::Delete { path: "src/lib.rs".into(), is_dir: false }]);
}

#[test]
fn delete_refuses_non_project_paths() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.delete_path("../outside.txt", false, window, cx));
    });
    assert!(!cx.has_pending_prompt(), "no confirm for a refused path");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.delete_path("/etc/passwd", false, window, cx));
    });
    assert!(!cx.has_pending_prompt(), "absolute path refused too");
    assert!(ISSUED.lock().is_empty(), "nothing dispatched");
}

#[test]
fn invalid_name_stays_armed_and_escape_cancels() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        ws.update(cx, |this, cx| this.begin_new_file("src", window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("explorer-input").visible());
    });
    cx.update(|window, cx| {
        window.input("../escape.rs", cx);
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).explorer.editing.is_some(), "invalid name keeps the input armed");
        assert!(ISSUED.lock().is_empty(), "no op dispatched");
    });
    // Escape on the still-armed input abandons the edit.
    cx.update(|window, cx| {
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).explorer.editing.is_none(), "Escape disarms the edit");
        assert!(window.try_find("explorer-input").is_none(), "input unmounts");
    });
}

#[test]
fn failed_op_surfaces_an_error_toast() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    *FAIL_WITH.lock() = Some("Permission denied".into());
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        ws.update(cx, |this, cx| this.begin_new_file("", window, cx));
        window.draw(cx).clear(cx);
        assert!(window.find("explorer-input").visible());
    });
    cx.update(|window, cx| {
        window.input("f.rs", cx);
        window.press("enter", cx);
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
