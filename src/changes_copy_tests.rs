//! Tests for "Copy Diff": `git::file_diff` against real temp repos (skipped
//! when git is unavailable) and the row-menu → clipboard path headless.

use std::path::{Path, PathBuf};

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

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

/// A temp git repo with `f.txt` = "one\n" committed — the ops need a HEAD.
/// Returns None when git isn't installed.
fn temp_repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-copydiff-{name}-{}", std::process::id()));
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

#[test]
fn file_diff_returns_worktree_hunks() {
    let Some(dir) = temp_repo("worktree") else { return };
    std::fs::write(dir.join("f.txt"), "new\n").unwrap();
    let diff = git::file_diff(&dir, "f.txt", false).unwrap();
    assert!(diff.contains("--- a/f.txt") && diff.contains("+++ b/f.txt"), "{diff}");
    assert!(diff.contains("-one") && diff.contains("+new"), "{diff}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_diff_staged_reads_the_cached_half() {
    let Some(dir) = temp_repo("staged") else { return };
    // Index holds "staged\n"; the worktree adds a line that stays unstaged.
    std::fs::write(dir.join("f.txt"), "staged\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    std::fs::write(dir.join("f.txt"), "staged\nunstaged\n").unwrap();
    let staged = git::file_diff(&dir, "f.txt", true).unwrap();
    assert!(staged.contains("-one") && staged.contains("+staged"), "{staged}");
    assert!(!staged.contains("unstaged"), "--cached must not see worktree-only lines: {staged}");
    let worktree = git::file_diff(&dir, "f.txt", false).unwrap();
    assert!(worktree.contains("+unstaged") && !worktree.contains("-one"), "{worktree}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_diff_synthesizes_new_file_patch_for_untracked() {
    let Some(dir) = temp_repo("untracked") else { return };
    std::fs::write(dir.join("new.txt"), "alpha\nbeta\n").unwrap();
    let diff = git::file_diff(&dir, "new.txt", false).unwrap();
    assert!(diff.contains("new file mode"), "{diff}");
    assert!(diff.contains("--- /dev/null") && diff.contains("+++ b/new.txt"), "{diff}");
    assert!(diff.contains("@@ -0,0 +1,2 @@"), "{diff}");
    assert!(diff.contains("+alpha") && diff.contains("+beta"), "{diff}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_diff_errors_on_missing_or_unchanged() {
    let Some(dir) = temp_repo("missing") else { return };
    assert!(git::file_diff(&dir, "gone.txt", false).is_err(), "untracked and unreadable");
    assert!(git::file_diff(&dir, "f.txt", false).is_err(), "tracked but unchanged");
    assert!(git::file_diff(&dir, "f.txt", true).is_err(), "nothing staged");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn change_row_menu_copies_diff_to_clipboard() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let Some(dir) = temp_repo("menu") else { return };
    std::fs::write(dir.join("f.txt"), "new\n").unwrap();
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.changes = vec![change("f.txt", ChangeStatus::Modified, 1, 1)];
            this.changes_panel_open = true;
            cx.notify();
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the file menu");
        // Click "Copy Diff" by label — the tracked-file git items sit between
        // it and "Copy Path", so a positional index would drift.
        let item = gpui_kit::base::test_support::snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Copy Diff"))
            .expect("menu should offer Copy Diff")
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
    });
    // The diff is fetched on the background executor — poll the clipboard.
    let mut clip = String::new();
    for _ in 0..200 {
        cx.run_until_parked();
        clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
        if !clip.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(clip.contains("-one") && clip.contains("+new"), "clipboard: {clip:?}");
    let note = ws.read_with(cx, |w, _| w.git.note.clone());
    assert_eq!(note, Some(("Copied diff for f.txt".to_string(), false)));
    let _ = std::fs::remove_dir_all(&dir);
}
