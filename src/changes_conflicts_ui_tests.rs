//! Headless UI tests for the Changes panel's conflicts section — the banner
//! count, the per-file action chips, and the real-git round trip where a
//! "Use ours" click clears the conflict. Same `TestAppContext::single()`
//! pattern as `changes_ui_tests.rs`.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, VisualTestContext};

use crate::changes_ui_tests::{change, mount};
use crate::git::ChangeStatus;
use crate::open_in::{ISSUED, PreferredEditor, open_command};
use crate::workspace::Workspace;

/// A temp repo mid-merge-conflict on `f.txt` — same fixture as
/// `changes_conflicts_tests`, duplicated so each test file stands alone.
/// Returns None when git isn't installed.
fn conflicted_repo(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-conflicts-ui-{name}-{}", std::process::id()));
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
    if !run(&["init", "-q", "-b", "main"]) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    assert!(run(&["config", "user.email", "t@t"]));
    assert!(run(&["config", "user.name", "t"]));
    std::fs::write(dir.join("f.txt"), "base\n").unwrap();
    assert!(run(&["add", "f.txt"]));
    assert!(run(&["commit", "-qm", "init"]));
    assert!(run(&["checkout", "-qb", "side"]));
    std::fs::write(dir.join("f.txt"), "theirs\n").unwrap();
    assert!(run(&["commit", "-qam", "side"]));
    assert!(run(&["checkout", "-q", "main"]));
    std::fs::write(dir.join("f.txt"), "ours\n").unwrap();
    assert!(run(&["commit", "-qam", "main"]));
    assert!(!run(&["merge", "side"]), "the fixture merge must conflict");
    Some(dir)
}

/// Poll until `cond` holds — git ops run on real threads, so
/// `run_until_parked` alone can return before the subprocess lands.
fn until(ws: &gpui_kit::Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..200 {
        cx.run_until_parked();
        if ws.read_with(cx, |w, _| cond(w)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("condition never held");
}

#[test]
fn conflicts_section_shows_banner_and_actions() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.conflicts = vec!["src/a.rs".into(), "src/b.rs".into()];
            this.changes = vec![
                change("src/a.rs", ChangeStatus::Conflicted, 0, 0),
                change("src/b.rs", ChangeStatus::Conflicted, 0, 0),
            ];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(window.find("conflicts-banner").label(), Some("2 conflicts — resolve to continue the merge"));
        for ix in 0..2usize {
            assert!(window.find(("conflict-row", ix)).visible(), "conflict row {ix} renders");
            assert!(window.find(("conflict-ours", ix)).visible(), "use-ours chip {ix}");
            assert!(window.find(("conflict-theirs", ix)).visible(), "use-theirs chip {ix}");
            assert!(window.find(("conflict-edit", ix)).visible(), "open-in-editor chip {ix}");
            assert!(window.find(("conflict-resolved", ix)).visible(), "mark-resolved chip {ix}");
        }
    });
}

#[test]
fn conflicts_section_hides_when_clean() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.conflicts = Vec::new();
            this.changes = vec![change("src/a.rs", ChangeStatus::Modified, 1, 0)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find("changes-panel").visible(), "panel renders");
        assert!(window.try_find("conflicts-section").is_none(), "no conflicts section on a clean tree");
    });
}

/// Clicking "Use ours" runs `checkout --ours` + `add` in the project root —
/// the refresh that lands after the op drops the row and the banner. Skips
/// when git is unavailable.
#[test]
fn use_ours_resolves_the_conflict() {
    let Some(dir) = conflicted_repo("ours") else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.toggle_changes_panel(cx);
        });
    });
    until(&ws, cx, |w| !w.git.conflicts.is_empty());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).git.conflicts, ["f.txt"]);
        assert!(window.find("conflicts-section").visible(), "section renders for the conflicted repo");
        window.click(("conflict-ours", 0usize), cx);
    });
    until(&ws, cx, |w| w.git.conflicts.is_empty() && w.git.note.is_some());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "ours\n");
        assert!(window.try_find("conflicts-section").is_none(), "section hides once resolved");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Mark resolved" runs `git add` — the hand-edited file stages and the
/// section empties. Skips when git is unavailable.
#[test]
fn mark_resolved_stages_the_edited_file() {
    let Some(dir) = conflicted_repo("mark") else { return };
    std::fs::write(dir.join("f.txt"), "merged\n").unwrap();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.toggle_changes_panel(cx);
        });
    });
    until(&ws, cx, |w| !w.git.conflicts.is_empty());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("conflict-resolved", 0usize), cx);
    });
    until(&ws, cx, |w| w.git.conflicts.is_empty() && w.git.note.is_some());
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "merged\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Open in editor" hands the file to the preferred editor — the recorded
/// command is `open -a <App> <abs>`. Skips when git is unavailable.
#[test]
fn open_in_editor_issues_the_open_command() {
    let Some(dir) = conflicted_repo("edit") else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.preferred_editor = PreferredEditor::VsCode;
            this.toggle_changes_panel(cx);
        });
    });
    until(&ws, cx, |w| !w.git.conflicts.is_empty());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("conflict-edit", 0usize), cx);
    });
    for _ in 0..200 {
        cx.run_until_parked();
        if !ISSUED.lock().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // `Project::open` canonicalizes — /var → /private/var on macOS.
    let abs = ws.read_with(cx, |w, _| w.project.root().join("f.txt"));
    let expected = open_command(PreferredEditor::VsCode, &abs).expect("VsCode builds a command");
    assert_eq!(ISSUED.lock().as_slice(), &[expected]);
}
