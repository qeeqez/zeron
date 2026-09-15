//! Headless UI tests for the Changes panel's "Recent commits" section —
//! listing commits, expanding a commit's `git show` diff, and the revert op.
//! Same `TestAppContext::single()` pattern as `changes_git_ui_tests.rs`.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::git::{BranchStatus, Commit};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-commits-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

fn commit(hash: &str, subject: &str) -> Commit {
    Commit {
        hash: hash.into(),
        subject: subject.into(),
        author: "Alice".into(),
        rel_time: "2 hours ago".into(),
        diff: None,
        diff_load: 0,
    }
}

fn branch() -> BranchStatus {
    BranchStatus { name: "main".into(), upstream: None, ahead: 0, behind: 0 }
}

/// A temp git repo with two commits — the log needs a HEAD with history.
/// Returns None when git isn't installed.
fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-commits-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !run(&["init", "-q"]) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    assert!(run(&["config", "user.email", "t@t"]));
    assert!(run(&["config", "user.name", "t"]));
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert!(run(&["add", "f.txt"]));
    assert!(run(&["commit", "-qm", "first"]));
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    assert!(run(&["add", "f.txt"]));
    assert!(run(&["commit", "-qm", "second"]));
    Some(dir)
}

#[test]
fn commits_section_lists_recent_commits() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.commits = vec![commit("abc1234", "second"), commit("def5678", "first")];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("commits-section").visible(), "section renders");
        assert!(window.find(("commit-row", 0usize)).visible(), "first commit renders");
        assert!(window.find(("commit-row", 1usize)).visible(), "second commit renders");
    });
}

#[test]
fn commits_section_hidden_when_empty() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.git.commits = vec![];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("changes-git").visible(), "git block renders");
        assert!(window.try_find("commits-section").is_none(), "no section without commits");
    });
}

/// Opening the panel on a real repo fills the section from `git log` on the
/// background executor; clicking a row expands that commit's `git show`
/// patch inline. Skips when git is unavailable.
#[test]
fn opening_panel_lists_commits_and_click_shows_diff() {
    let Some(dir) = temp_repo("expand") else { return };
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
        let commits = &ws.read(cx).git.commits;
        assert_eq!(commits.len(), 2, "refresh collected the log");
        assert_eq!(commits[0].subject, "second", "newest first");
        assert!(window.find("commits-section").visible(), "section renders");
        assert!(window.try_find(("commit-diff", 0usize)).is_none(), "diff starts collapsed");

        window.click(("commit-row", 0usize), cx);
        assert!(ws.read(cx).git.commits[0].diff.is_none(), "diff load is off the click path");
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("commit-diff", 0usize)).visible(), "click expands the commit diff");
        let diff = ws.read(cx).git.commits[0].diff.as_ref().expect("diff cached on the row");
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].path, "f.txt");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// The context menu's Revert runs `git revert --no-edit` — the refresh that
/// lands after the op re-lists commits with the revert on top. Skips when
/// git is unavailable.
#[test]
fn revert_commit_adds_a_reverting_commit() {
    let Some(dir) = temp_repo("revert") else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.toggle_changes_panel(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, cx| {
        let sha = ws.read(cx).git.commits[0].hash.clone();
        ws.update(cx, |this, cx| this.revert_commit(&sha, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let commits = &ws.read(cx).git.commits;
        assert_eq!(commits.len(), 3, "revert added a commit");
        assert!(commits[0].subject.contains("Revert"), "newest commit is the revert");
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\n", "revert restored the content");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// A commit-diff load stamped with an older generation must not publish —
/// same stale-result guard as the file rows.
#[test]
fn stale_commit_diff_is_discarded() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.git.commits = vec![commit("abc1234", "second")];
            let stale = (this.changes_generation, 1);
            this.changes_generation += 1; // a newer refresh was requested
            this.land_commit_diff(stale, Some(crate::git::CommitDiff::default()), cx);
            assert!(this.git.commits[0].diff.is_none(), "stale diff did not publish");
        });
    });
}
