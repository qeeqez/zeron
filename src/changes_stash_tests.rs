//! Tests for the Changes panel's stash support — `git stash` ops against
//! real temp repos (skipped when git is unavailable) and headless rendering
//! of the stash section. Declared as `crate::changes_stash::tests` via
//! `#[path]` so `main.rs` stays under the SLOC cap.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use crate::changes_ui_tests::{change, mount};
use crate::git::{self, BranchStatus, ChangeStatus, StashEntry};

fn run(dir: &std::path::Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A temp git repo with one commit and a local identity — `stash push` needs
/// a HEAD and a `user.*`. Returns None when git isn't installed.
fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-stash-{name}-{}", std::process::id()));
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

fn branch() -> BranchStatus {
    BranchStatus {
        name: "main".into(),
        upstream: Some("origin/main".into()),
        ahead: 0,
        behind: 0,
    }
}

#[test]
fn stash_push_saves_changes_and_clears_the_worktree() {
    let Some(dir) = temp_repo("push") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("new.txt"), "untracked\n").unwrap();
    let note = git::stash_push(&dir, "my work").unwrap();
    assert!(note.contains("my work"), "note carries the message: {note}");
    assert!(git::collect(&dir).is_empty(), "stash left a clean worktree");
    let stashes = git::stash_list(&dir);
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].name, "stash@{0}");
    assert!(stashes[0].message.contains("my work"), "message parsed: {}", stashes[0].message);
    assert!(!stashes[0].rel_time.is_empty(), "age parsed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stash_push_on_a_clean_tree_reports_no_changes() {
    let Some(dir) = temp_repo("clean") else { return };
    let note = git::stash_push(&dir, "WIP").unwrap();
    assert!(note.contains("No local changes"), "clean tree is a note, not an error: {note}");
    assert!(git::stash_list(&dir).is_empty(), "nothing was stashed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stash_list_parses_entries_newest_first() {
    let Some(dir) = temp_repo("list") else { return };
    std::fs::write(dir.join("f.txt"), "two\n").unwrap();
    git::stash_push(&dir, "first").unwrap();
    std::fs::write(dir.join("f.txt"), "three\n").unwrap();
    git::stash_push(&dir, "second").unwrap();
    let stashes = git::stash_list(&dir);
    assert_eq!(stashes.len(), 2);
    assert_eq!(stashes[0].name, "stash@{0}");
    assert!(stashes[0].message.contains("second"), "newest first: {}", stashes[0].message);
    assert_eq!(stashes[1].name, "stash@{1}");
    assert!(stashes[1].message.contains("first"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stash_pop_restores_changes_and_removes_the_entry() {
    let Some(dir) = temp_repo("pop") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    git::stash_push(&dir, "wip").unwrap();
    assert!(git::collect(&dir).is_empty());
    git::stash_pop(&dir, "stash@{0}").unwrap();
    assert!(git::stash_list(&dir).is_empty(), "pop dropped the entry");
    let changes = git::collect(&dir);
    assert_eq!(changes.len(), 1, "the edit is back in the worktree");
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\ntwo\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stash_apply_restores_but_keeps_the_entry() {
    let Some(dir) = temp_repo("apply") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    git::stash_push(&dir, "wip").unwrap();
    git::stash_apply(&dir, "stash@{0}").unwrap();
    assert_eq!(git::stash_list(&dir).len(), 1, "apply kept the entry");
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\ntwo\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stash_drop_removes_the_entry_without_applying() {
    let Some(dir) = temp_repo("drop") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    git::stash_push(&dir, "wip").unwrap();
    git::stash_drop(&dir, "stash@{0}").unwrap();
    assert!(git::stash_list(&dir).is_empty(), "drop removed the entry");
    assert!(git::collect(&dir).is_empty(), "drop did not touch the worktree");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The stash section renders its input + button whenever the repo block is
/// up; stash rows list each entry's selector, message, and age.
#[test]
fn stash_section_renders_input_button_and_rows() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.stashes = vec![
                StashEntry {
                    name: "stash@{0}".into(),
                    message: "On main: wip".into(),
                    rel_time: "2h ago".into(),
                },
                StashEntry {
                    name: "stash@{1}".into(),
                    message: "WIP on main: old".into(),
                    rel_time: "3d ago".into(),
                },
            ];
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, 1, 0)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("stash-section").visible(), "stash section renders");
        assert!(window.find("stash-button").visible(), "stash button renders");
        assert!(window.find(("stash-row", 0usize)).visible(), "first stash row renders");
        assert!(window.find(("stash-row", 1usize)).visible(), "second stash row renders");
    });
}

/// Clicking Stash runs `git stash push -u` — the worktree clears and the
/// entry lands in the list. Skips when git is unavailable.
#[test]
fn stash_button_stashes_the_worktree() {
    let Some(dir) = temp_repo("button") else { return };
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
        window.click("stash-button", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).changes.is_empty(), "stash cleared the worktree");
        assert_eq!(ws.read(cx).git.stashes.len(), 1, "the entry landed in the list");
        assert!(window.find(("stash-row", 0usize)).visible(), "stash row renders");
        assert!(ws.read(cx).git.note.is_some(), "op landed a note");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Right-clicking a stash row opens its menu; Pop restores the changes and
/// removes the entry. Skips when git is unavailable.
#[test]
fn stash_row_menu_pop_restores_the_changes() {
    let Some(dir) = temp_repo("menu") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    git::stash_push(&dir, "wip").unwrap();
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
        window.right_click(("stash-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click opened the stash menu");
        window.within("popup-menu").click(0usize, cx); // Pop
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).git.stashes.is_empty(), "pop removed the entry");
        assert_eq!(ws.read(cx).changes.len(), 1, "the edit is back in the list");
    });
    let _ = std::fs::remove_dir_all(&dir);
}
