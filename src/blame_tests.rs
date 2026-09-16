//! Tests for `git::blame` and `git::file_log` against real temp repos
//! (skipped when git is unavailable), plus the helpers the UI tests in
//! `blame_ui_tests.rs` share. Declared as `crate::git::blame::blame_tests`
//! via `#[path]` so `main.rs` stays under the SLOC cap.

use std::path::{Path, PathBuf};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, Entity, TestAppContext, VisualTestContext, Window};

use crate::changes_ui_tests::{change, mount};
use crate::git::{self, ChangeStatus};
use crate::workspace::Workspace;

pub(crate) fn run(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `git -c user.name=<name> -c user.email=<name>@t commit -qm <msg>` — the
/// two-author blame test needs per-commit identities.
pub(crate) fn commit_as(dir: &Path, name: &str, msg: &str) -> bool {
    run(dir, &["-c", &format!("user.name={name}"), "-c", &format!("user.email={name}@t"), "commit", "-qm", msg])
}

/// A temp git repo with a local identity. Returns None when git isn't
/// installed.
pub(crate) fn temp_repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-blame-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if !run(&dir, &["init", "-q"]) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    assert!(run(&dir, &["config", "user.email", "t@t"]));
    assert!(run(&dir, &["config", "user.name", "t"]));
    Some(dir)
}

/// Advance the test clock until `cond` holds or the budget runs out — the
/// blame/history fetch runs on the background executor, so a bare
/// `run_until_parked` can miss it.
pub(crate) fn until(ws: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..200 {
        cx.run_until_parked();
        if ws.read_with(cx, |w, _| cond(w)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(ws.read_with(cx, |w, _| cond(w)), "condition never held");
}

pub(crate) fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window).iter().filter_map(|s| s.label().map(|l| l.to_string())).collect()
}

/// Click the popup-menu item with `label` — panics when it isn't offered.
pub(crate) fn click_menu_item(window: &mut Window, label: &str, cx: &mut App) {
    let item = snapshots(window)
        .iter()
        .find(|s| s.label() == Some(label))
        .unwrap_or_else(|| panic!("menu should offer {label}"))
        .clone();
    let id = item.path().last().unwrap().clone();
    window.within("popup-menu").click(id, cx);
}

/// A mounted workspace rooted at `dir` with one Changes row for `path`.
pub(crate) fn mounted<'a>(app: &'a mut TestAppContext, dir: &Path, path: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (ws, cx) = mount(app);
    let path = path.to_string();
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(dir);
            this.changes = vec![change(&path, ChangeStatus::Modified, 1, 0)];
            this.changes_panel_open = true;
            cx.notify();
        });
    });
    (ws, cx)
}

#[test]
fn blame_attributes_lines_to_their_authors() {
    let Some(dir) = temp_repo("authors") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "alice", "first"));
    std::fs::write(dir.join("f.txt"), "one\nTWO\nthree\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "bob", "second"));

    let lines = git::blame(&dir, "f.txt").unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].author, "alice");
    assert_eq!(lines[0].summary, "first");
    assert_eq!(lines[0].text, "one");
    assert_eq!(lines[0].line_no, 1);
    // Line 2 was rewritten by bob's commit; line 3 is his addition.
    assert_eq!(lines[1].author, "bob");
    assert_eq!(lines[1].summary, "second");
    assert_eq!(lines[1].text, "TWO");
    assert_eq!(lines[2].author, "bob");
    assert_eq!(lines[2].text, "three");
    // The two commits carry distinct full hashes — the row copies this sha.
    assert_ne!(lines[0].sha, lines[1].sha);
    assert!(lines[0].sha.len() >= 40, "porcelain emits full ids: {}", lines[0].sha);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Untracked, missing, and non-repo paths all fail — git's stderr is the
/// error the overlay shows.
#[test]
fn blame_and_log_err_where_git_has_nothing() {
    let Some(dir) = temp_repo("untracked") else { return };
    std::fs::write(dir.join("new.txt"), "x\n").unwrap();
    assert!(git::blame(&dir, "new.txt").is_err(), "untracked file has no blame");
    assert!(git::blame(&dir, "absent.txt").is_err(), "missing file has no blame");
    let _ = std::fs::remove_dir_all(&dir);

    let dir = std::env::temp_dir().join(format!("rixlcode-blame-norepo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "x\n").unwrap();
    assert!(git::file_log(&dir, "f.txt").is_err(), "non-repo has no log");
    assert!(git::blame(&dir, "f.txt").is_err(), "non-repo has no blame");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_log_follows_renames() {
    let Some(dir) = temp_repo("rename") else { return };
    std::fs::write(dir.join("old.txt"), "one\n").unwrap();
    assert!(run(&dir, &["add", "old.txt"]));
    assert!(commit_as(&dir, "alice", "add old"));
    assert!(run(&dir, &["mv", "old.txt", "new.txt"]));
    assert!(commit_as(&dir, "bob", "rename to new"));
    std::fs::write(dir.join("new.txt"), "one\ntwo\n").unwrap();
    assert!(run(&dir, &["add", "new.txt"]));
    assert!(commit_as(&dir, "alice", "extend new"));

    let commits = git::file_log(&dir, "new.txt").unwrap();
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["extend new", "rename to new", "add old"], "--follow kept the pre-rename commit");
    assert_eq!(commits[0].author, "alice");
    assert_eq!(commits[1].author, "bob");
    let _ = std::fs::remove_dir_all(&dir);
}
