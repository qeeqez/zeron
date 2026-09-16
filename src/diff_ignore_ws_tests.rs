//! Tests for the Changes panel's ignore-whitespace diff filter: real-repo
//! `diff_for_file` runs plus a headless click-through of the header chip.
//! Sibling file so `changes_diff_tests.rs`/`changes_ui_tests.rs` stay under
//! the SLOC cap.

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;

    use crate::changes_diff::{DiffLineKind, diff_for_file};
    use crate::changes_ui_tests::{change, mount};
    use crate::git::ChangeStatus;
    use crate::workspace::Workspace;

    /// Run `git` in `dir`; true on exit 0. Real-repo tests skip when it fails.
    fn git_ok(dir: &std::path::Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// A temp repo with `code.rs` committed, then reindented — the only
    /// change is leading whitespace. `None` when git can't run.
    fn ws_only_repo(name: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if !git_ok(&dir, &["init", "-q"]) {
            return None;
        }
        std::fs::write(dir.join("code.rs"), "fn f() {\n    a();\n}\n").unwrap();
        if !git_ok(&dir, &["add", "code.rs"]) || !git_ok(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]) {
            return None;
        }
        std::fs::write(dir.join("code.rs"), "fn f() {\n        a();\n}\n").unwrap();
        Some(dir)
    }

    /// A whitespace-only edit produces a diff normally and none under
    /// `--ignore-all-space`. Skips when `git` is unavailable.
    #[test]
    fn whitespace_only_change_hides_with_ignore_ws() {
        let Some(dir) = ws_only_repo("diffws") else { return };
        let change = change("code.rs", ChangeStatus::Modified, 1, 1);
        let plain = diff_for_file(&dir, &change, false).unwrap();
        assert!(plain.lines.iter().any(|l| l.kind == DiffLineKind::Removed), "plain diff shows the reindent");
        let ignored = diff_for_file(&dir, &change, true).unwrap();
        assert!(ignored.lines.is_empty(), "ignore-all-space collapses the reindent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same collapse when the reindent is staged — `diff HEAD` covers the
    /// index, so the flag applies to staged changes too.
    #[test]
    fn staged_whitespace_only_change_hides_with_ignore_ws() {
        let Some(dir) = ws_only_repo("diffws-staged") else { return };
        if !git_ok(&dir, &["add", "code.rs"]) {
            return;
        }
        let mut change = change("code.rs", ChangeStatus::Modified, 1, 1);
        change.staged = true;
        let ignored = diff_for_file(&dir, &change, true).unwrap();
        assert!(ignored.lines.is_empty(), "staged reindent collapses too");
        let plain = diff_for_file(&dir, &change, false).unwrap();
        assert!(!plain.lines.is_empty(), "staged reindent shows without the flag");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real content change still shows under `--ignore-all-space` — the
    /// flag filters whitespace, not edits.
    #[test]
    fn real_change_still_shows_with_ignore_ws() {
        let Some(dir) = ws_only_repo("diffws-real") else { return };
        std::fs::write(dir.join("code.rs"), "fn f() {\n    b();\n}\n").unwrap();
        let change = change("code.rs", ChangeStatus::Modified, 1, 1);
        let ignored = diff_for_file(&dir, &change, true).unwrap();
        assert!(ignored.lines.iter().any(|l| l.kind == DiffLineKind::Added), "content change survives the filter");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The header chip flips `git.ignore_ws`, persists it, and re-fetches the
    /// expanded row's diff — a whitespace-only edit renders its "no textual
    /// diff" placeholder while the filter is on and its lines when off.
    /// Skips when `git` is unavailable.
    #[test]
    fn header_chip_toggles_and_refetches_diff() {
        let Some(dir) = ws_only_repo("diffws-ui") else { return };
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        cx.update(|window, cx| {
            ws.update(cx, |this, cx| {
                this.project = crate::project::Project::open(&dir);
                this.changes = vec![change("code.rs", ChangeStatus::Modified, 1, 1)];
                this.changes_panel_open = true;
                cx.notify();
            });
            window.draw(cx).clear(cx);
            assert!(window.find("diff-ignore-ws").visible(), "chip renders in the header");
            assert!(!ws.read(cx).git.ignore_ws, "filter starts off");

            // Expand the row; the diff load lands off the click path.
            window.click(("change-row", 0usize), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(ws.read(cx).changes[0].diff.as_ref().is_some_and(|d| !d.lines.is_empty()), "reindent shows with the filter off");

            window.click("diff-ignore-ws", cx);
            assert!(ws.read(cx).git.ignore_ws, "click flips the flag");
            assert!(ws.read(cx).changes[0].diff_load != 0, "expanded row re-fetches");
        });
        assert!(crate::persist::load_settings().diff_ignore_ws, "toggle writes settings.json");

        // A second window on the same HOME loads the persisted preference.
        let (ws2, _cx2) = cx.add_window_view(Workspace::new);
        assert!(ws2.read_with(cx, |ws, _| ws.git.ignore_ws), "new window loads the persisted flag");

        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let diff = ws.read(cx).changes[0].diff.as_ref().unwrap();
            assert!(diff.lines.is_empty(), "reindent collapses under the filter");
            assert!(window.find(("change-diff", 0usize)).visible(), "diff body still renders");

            window.click("diff-ignore-ws", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(!ws.read(cx).git.ignore_ws, "second click flips back");
            let diff = ws.read(cx).changes[0].diff.as_ref().unwrap();
            assert!(!diff.lines.is_empty(), "reindent shows again");
        });
        assert!(!crate::persist::load_settings().diff_ignore_ws, "toggling back persists too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
