//! Tests for the Changes panel's per-file "Discard changes" —
//! `git::discard_file` against real temp repos (skipped when git is
//! unavailable), the row menu's item, and the headless confirm flow.
//! Declared as `crate::changes::changes_discard_tests` via `#[path]` so
//! `main.rs` stays under the SLOC cap.

use std::path::{Path, PathBuf};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, TestAppContext, Window};

use crate::changes_ui_tests::{change, mount};
use crate::git::{self, ChangeStatus};

fn run(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A temp git repo with one commit and a local identity — discarding staged
/// changes needs a HEAD to restore from. Returns None when git isn't
/// installed.
fn temp_repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-discard-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if !run(&dir, &["init", "-q"]) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    assert!(run(&dir, &["config", "user.email", "t@t"]));
    assert!(run(&dir, &["config", "user.name", "t"]));
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(run(&dir, &["commit", "-qm", "init"]));
    Some(dir)
}

/// The one collected `FileChange` for `path` — the same value the panel's
/// row menu hands to `discard_change`.
fn collected(dir: &Path, path: &str) -> git::FileChange {
    git::collect(dir)
        .into_iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("{path} should be a change"))
}

fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window).iter().filter_map(|s| s.label().map(|l| l.to_string())).collect()
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

#[test]
fn discard_restores_a_modified_tracked_file() {
    let Some(dir) = temp_repo("modified") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("other.txt"), "untouched\n").unwrap();
    let change = collected(&dir, "f.txt");
    assert_eq!(change.status, ChangeStatus::Modified);
    git::discard_file(&dir, &change).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\n");
    let rest = git::collect(&dir);
    assert_eq!(rest.len(), 1, "only the untracked neighbor remains");
    assert_eq!(rest[0].path, "other.txt", "other files are untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discard_deletes_an_untracked_file() {
    let Some(dir) = temp_repo("untracked") else { return };
    std::fs::write(dir.join("new.txt"), "untracked\n").unwrap();
    let change = collected(&dir, "new.txt");
    assert_eq!(change.status, ChangeStatus::Added);
    assert!(!change.staged, "untracked files report staged=false");
    git::discard_file(&dir, &change).unwrap();
    assert!(!dir.join("new.txt").exists(), "untracked file deleted");
    assert!(git::collect(&dir).is_empty(), "status clean for that path");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discard_restores_a_staged_modification() {
    let Some(dir) = temp_repo("staged") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    let change = collected(&dir, "f.txt");
    assert!(change.staged);
    git::discard_file(&dir, &change).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\n");
    assert!(git::collect(&dir).is_empty(), "gone from index and worktree");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discard_removes_a_staged_new_file() {
    let Some(dir) = temp_repo("staged-new") else { return };
    std::fs::write(dir.join("new.txt"), "added\n").unwrap();
    assert!(run(&dir, &["add", "new.txt"]));
    let change = collected(&dir, "new.txt");
    assert_eq!(change.status, ChangeStatus::Added);
    assert!(change.staged);
    git::discard_file(&dir, &change).unwrap();
    assert!(!dir.join("new.txt").exists(), "staged-new file deleted");
    assert!(git::collect(&dir).is_empty(), "gone from index and worktree");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discard_restores_a_staged_rename() {
    let Some(dir) = temp_repo("rename") else { return };
    assert!(run(&dir, &["mv", "f.txt", "renamed.txt"]));
    let change = collected(&dir, "renamed.txt");
    assert_eq!(change.status, ChangeStatus::Renamed);
    assert_eq!(change.source.as_deref(), Some("f.txt"));
    git::discard_file(&dir, &change).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\n", "source restored");
    assert!(!dir.join("renamed.txt").exists(), "rename target deleted");
    assert!(git::collect(&dir).is_empty(), "no staged deletion left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Changes row's right-click menu appends "Discard Changes…" after the
/// shared file items; the explorer's menu — same `file_menu` — doesn't.
#[test]
fn discard_item_only_on_changes_rows() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, 1, 0)];
            this.changes_panel_open = true;
            this.project_files = vec!["README.md".into()];
            this.set_sidebar_tab(crate::views::sidebar::SidebarTab::Files, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Reveal in Finder"), "shared file items render");
        assert!(labels.iter().any(|l| l == "Discard Changes…"), "changes row offers discard: {labels:?}");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("explorer-file", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Reveal in Finder"), "explorer menu opened");
        assert!(!labels.iter().any(|l| l == "Discard Changes…"), "explorer row has no discard: {labels:?}");
    });
}

/// Choosing "Discard Changes…" asks for confirmation first; confirming runs
/// the op and the file reverts on disk. Skips when git is unavailable.
#[test]
fn discard_flow_confirms_then_restores_the_file() {
    let Some(dir) = temp_repo("flow") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.toggle_changes_panel(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).changes.len(), 1, "edit shows as a change");
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Discard Changes…", cx);
    });
    // `cx`'s borrow of `app` ends here — the prompt helpers live on `app`.
    assert!(app.has_pending_prompt(), "discard asks for confirmation");
    app.simulate_prompt_answer("Discard");
    app.run_until_parked();
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\n", "file reverted on disk");
    app.update(|cx| assert!(ws.read(cx).changes.is_empty(), "refresh dropped the row"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cancelling the confirm leaves the file and the row untouched.
#[test]
fn discard_cancel_leaves_the_file_alone() {
    let Some(dir) = temp_repo("cancel") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.toggle_changes_panel(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Discard Changes…", cx);
    });
    assert!(app.has_pending_prompt(), "discard asks for confirmation");
    app.simulate_prompt_answer("Cancel");
    app.run_until_parked();
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\ntwo\n", "edit survived the cancel");
    app.update(|cx| assert_eq!(ws.read(cx).changes.len(), 1, "row still listed"));
    let _ = std::fs::remove_dir_all(&dir);
}
