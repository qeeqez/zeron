//! Headless UI tests for the Changes panel's git actions — stage toggles,
//! the commit box, and the push/PR block. Same `TestAppContext::single()`
//! pattern as `changes_ui_tests.rs`.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::git::{BranchStatus, ChangeStatus, FileChange};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-changes-git-test-{}", std::process::id()));
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

/// A temp git repo with one commit — the ops need a HEAD. Returns None when
/// git isn't installed.
fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-changes-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .map(|o| o.status.success())
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
    assert!(run(&["commit", "-qm", "init"]));
    Some(dir)
}

#[test]
fn git_block_shows_branch_commit_box_and_buttons() {
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
        assert!(window.find("changes-git").visible(), "git block renders");
        assert!(window.find("git-branch").visible(), "branch row renders");
        assert!(window.find("commit-button").visible(), "commit button renders");
        assert!(window.find("push-button").visible(), "push button renders");
        assert!(window.find("create-pr-button").visible(), "create-PR button renders");
        assert!(window.find(("stage-toggle", 0usize)).visible(), "stage toggle renders on the row");
    });
}

#[test]
fn non_repo_hides_the_git_actions() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = None;
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, false)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("changes-panel").visible(), "panel still renders");
        assert!(window.try_find("changes-git").is_none(), "no git block without a repo");
        assert!(window.try_find(("stage-toggle", 0usize)).is_none(), "no stage toggle without a repo");
    });
}

/// Clicking a row's stage toggle runs `git add` in the project root — the
/// refresh that lands after the op marks the row staged. Skips when git is
/// unavailable.
#[test]
fn stage_toggle_stages_the_file() {
    let Some(dir) = temp_repo("stage") else { return };
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
        assert!(!ws.read(cx).changes[0].staged, "file starts unstaged");
        window.click(("stage-toggle", 0usize), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).changes[0].staged, "git add staged the file");
        assert!(ws.read(cx).git.note.is_some(), "op landed a note");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Typing a message and clicking Commit runs `git commit` — the refresh
/// clears the row and the box empties. Skips when git is unavailable.
#[test]
fn commit_button_commits_staged_files() {
    let Some(dir) = temp_repo("commit") else { return };
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
        ws.update(cx, |this, cx| {
            crate::git::stage(this.project.root(), "f.txt").unwrap();
            this.git.commit_input.update(cx, |s, cx| s.set_value("second commit", window, cx));
            this.refresh_changes(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).changes[0].staged);
        window.click("commit-button", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).changes.is_empty(), "commit left a clean tree");
        assert!(ws.read(cx).git.commit_input.read(cx).value().is_empty(), "commit cleared the box");
        let note = &ws.read(cx).git.note;
        assert_eq!(note.as_ref().map(|(t, e)| (t.as_str(), *e)), Some(("Committed", false)));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty commit message refuses to spawn an op — no busy flag, no note.
#[test]
fn empty_commit_message_is_refused() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.git.branch = Some(branch());
            this.commit_staged(cx);
            assert!(!this.git.busy, "empty message did not start an op");
            assert!(this.git.note.is_none());
        });
    });
}
