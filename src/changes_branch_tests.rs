//! Tests for the Changes panel's branch ops — rename, delete, fetch, pull.
//! The `git::*` fns run against real temp repos (skipped when git is
//! unavailable); the picker menu, header buttons, and the rename/delete
//! flows get headless UI coverage via `TestAppContext::single()`.

use std::path::{Path, PathBuf};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, ElementId, Entity, SharedString, TestAppContext, VisualTestContext, Window};

use crate::git::{self, BranchStatus, ChangeStatus, FileChange};
use crate::workspace::Workspace;

fn run(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A temp git repo with one commit and a local identity — the ops need a
/// HEAD and `commit` needs `user.*`. Returns None when git isn't installed.
fn temp_repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-branch-{name}-{}", std::process::id()));
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

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-branch-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

fn change(path: &str, status: ChangeStatus, staged: bool) -> FileChange {
    FileChange {
        path: path.into(),
        source: None,
        status,
        staged,
        added: 1,
        deleted: 0,
        diff: None,
        diff_load: 0,
    }
}

fn branch() -> BranchStatus {
    BranchStatus {
        name: "main".into(),
        upstream: Some("origin/main".into()),
        ahead: 2,
        behind: 1,
    }
}

/// Open the Changes panel on `dir`, open the branch picker, and right-click
/// `row` — the shared lead-in for the menu tests.
fn open_picker_and_right_click(ws: &Entity<Workspace>, cx: &mut VisualTestContext, dir: &Path, row: ElementId) {
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(dir);
            this.toggle_changes_panel(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("git-branch", cx);
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(row, cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
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
fn rename_branch_renames_including_the_current_one() {
    let Some(dir) = temp_repo("rename") else { return };
    assert!(run(&dir, &["branch", "feature"]));

    git::rename_branch(&dir, "feature", "renamed").unwrap();
    let branches = git::list_branches(&dir);
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"renamed"), "new name listed: {names:?}");
    assert!(!names.contains(&"feature"), "old name gone: {names:?}");

    // `git branch -m` works on the checked-out branch too.
    let current = git::branch_status(&dir).unwrap().name;
    git::rename_branch(&dir, &current, "main-renamed").unwrap();
    assert_eq!(git::branch_status(&dir).unwrap().name, "main-renamed");

    // An existing target name is refused.
    assert!(git::rename_branch(&dir, "renamed", "main-renamed").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_branch_removes_merged_and_refuses_unmerged() {
    let Some(dir) = temp_repo("delete") else { return };
    let initial = git::branch_status(&dir).unwrap().name;

    // A merged branch deletes cleanly.
    assert!(run(&dir, &["branch", "merged"]));
    git::delete_branch(&dir, "merged").unwrap();
    assert!(!git::list_branches(&dir).iter().any(|b| b.name == "merged"), "merged branch deleted");

    // An unmerged branch is refused — git's stderr is the error, nothing forced.
    assert!(run(&dir, &["checkout", "-qb", "unmerged"]));
    std::fs::write(dir.join("u.txt"), "u\n").unwrap();
    assert!(run(&dir, &["add", "u.txt"]));
    assert!(run(&dir, &["commit", "-qm", "u"]));
    assert!(run(&dir, &["checkout", "-q", &initial]));
    let err = git::delete_branch(&dir, "unmerged").unwrap_err();
    assert!(err.to_lowercase().contains("not fully merged"), "git's refusal is the note: {err}");
    assert!(git::list_branches(&dir).iter().any(|b| b.name == "unmerged"), "unmerged branch survives");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `fetch` picks up a remote commit (behind count moves) and `pull_ff`
/// fast-forwards it; a diverged pull is refused rather than merged.
#[test]
fn fetch_and_pull_ff_track_a_remote() {
    let Some(dir) = temp_repo("fetch") else { return };
    let remote = std::env::temp_dir().join(format!("rixlcode-branch-remote-{}", std::process::id()));
    let other = std::env::temp_dir().join(format!("rixlcode-branch-other-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&other);
    assert!(run(&dir, &["init", "-q", "--bare", &remote.to_string_lossy()]));
    assert!(run(&dir, &["remote", "add", "origin", &remote.to_string_lossy()]));
    git::push(&dir).unwrap();

    // A second clone commits to the remote; `dir` doesn't know yet.
    assert!(run(&dir, &["clone", "-q", &remote.to_string_lossy(), &other.to_string_lossy()]));
    assert!(run(&other, &["config", "user.email", "t@t"]));
    assert!(run(&other, &["config", "user.name", "t"]));
    std::fs::write(other.join("two.txt"), "two\n").unwrap();
    assert!(run(&other, &["add", "two.txt"]));
    assert!(run(&other, &["commit", "-qm", "two"]));
    assert!(run(&other, &["push", "-q"]));

    git::fetch(&dir).unwrap();
    assert_eq!(git::branch_status(&dir).unwrap().behind, 1, "fetch saw the remote commit");
    git::pull_ff(&dir).unwrap();
    let branch = git::branch_status(&dir).unwrap();
    assert_eq!((branch.ahead, branch.behind), (0, 0), "pull fast-forwarded");
    assert!(dir.join("two.txt").is_file(), "pull brought the file");

    // Diverged: a local commit plus another remote commit — ff-only refuses.
    std::fs::write(dir.join("local.txt"), "l\n").unwrap();
    assert!(run(&dir, &["add", "local.txt"]));
    assert!(run(&dir, &["commit", "-qm", "local"]));
    std::fs::write(other.join("three.txt"), "three\n").unwrap();
    assert!(run(&other, &["add", "three.txt"]));
    assert!(run(&other, &["commit", "-qm", "three"]));
    assert!(run(&other, &["push", "-q"]));
    git::fetch(&dir).unwrap();
    assert!(git::pull_ff(&dir).is_err(), "diverged pull refuses to merge");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&other);
}

/// The branch header shows Fetch and Pull buttons beside the ahead/behind
/// badges.
#[test]
fn branch_row_shows_fetch_and_pull_buttons() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, false)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("fetch-button").visible(), "fetch button renders");
        assert!(window.find("pull-button").visible(), "pull button renders");
    });
}

/// Right-clicking a picker row offers Rename and Delete; the current
/// branch's row keeps Rename but never offers Delete.
#[test]
fn picker_row_menu_offers_rename_and_delete() {
    let Some(dir) = temp_repo("menu") else { return };
    let current = git::branch_status(&dir).unwrap().name;
    assert!(run(&dir, &["branch", "feature"]));
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_picker_and_right_click(&ws, cx, &dir, "branch-opt-feature".into());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "row menu opened");
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Rename…"), "menu offers Rename…: {labels:?}");
        assert!(labels.iter().any(|l| l == "Delete"), "menu offers Delete: {labels:?}");
        window.press("escape", cx);
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| {
        window.right_click(SharedString::from(format!("branch-opt-{current}")), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Rename…"), "current row still offers Rename…");
        assert!(!labels.iter().any(|l| l == "Delete"), "current row never offers Delete");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Choosing "Rename…" arms the header's rename input (the picker closes on
/// the menu click); confirming runs `git branch -m`.
#[test]
fn rename_flow_renames_the_branch() {
    let Some(dir) = temp_repo("rename-ui") else { return };
    assert!(run(&dir, &["branch", "feature"]));
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_picker_and_right_click(&ws, cx, &dir, "branch-opt-feature".into());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Rename…", cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).git.rename_target.as_deref(), Some("feature"), "rename armed for feature");
        assert!(window.find("rename-branch-input").visible(), "header shows the rename input");
        ws.update(cx, |this, cx| {
            this.git.rename_input.update(cx, |s, cx| s.set_value("renamed", window, cx));
        });
        window.draw(cx).clear(cx);
        window.click("rename-branch-confirm", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).git.rename_target.is_none(), "rename disarmed after success");
        assert!(git::list_branches(&dir).iter().any(|b| b.name == "renamed"), "branch renamed on disk");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Choosing "Delete" asks for confirmation first; confirming runs
/// `git branch -d` and the branch is gone.
#[test]
fn delete_flow_confirms_then_removes_the_branch() {
    let Some(dir) = temp_repo("delete-ui") else { return };
    assert!(run(&dir, &["branch", "feature"]));
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_picker_and_right_click(&ws, cx, &dir, "branch-opt-feature".into());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Delete", cx);
    });
    // `cx`'s borrow of `app` ends here — the prompt helpers live on `app`.
    assert!(app.has_pending_prompt(), "delete asks for confirmation");
    app.simulate_prompt_answer("Delete");
    app.run_until_parked();
    assert!(!git::list_branches(&dir).iter().any(|b| b.name == "feature"), "branch deleted");
    let _ = std::fs::remove_dir_all(&dir);
}
