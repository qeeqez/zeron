//! Headless UI tests for the Changes panel — same `TestAppContext::single()`
//! pattern as `ui_tests.rs` (the `#[gpui_kit::test]` macro and a bare
//! `use gpui_kit::*` crash the proc-macro on this nightly).

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, KeyBinding, TestAppContext, VisualTestContext};

use crate::changes_diff::{DiffLine, DiffLineKind, FileDiff};
use crate::git::{ChangeStatus, FileChange};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-changes-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

fn change(path: &str, status: ChangeStatus, added: u32, deleted: u32) -> FileChange {
    FileChange {
        path: path.into(),
        source: None,
        status,
        added,
        deleted,
        diff: None,
        staged: false,
        diff_load: 0,
    }
}

fn sample_diff() -> FileDiff {
    FileDiff {
        lines: vec![
            DiffLine {
                kind: DiffLineKind::Hunk,
                old: None,
                new: None,
                text: "@@ -1,3 +1,4 @@".into(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                old: Some(1),
                new: Some(1),
                text: "fn main() {".into(),
            },
            DiffLine {
                kind: DiffLineKind::Removed,
                old: Some(2),
                new: None,
                text: "old();".into(),
            },
            DiffLine {
                kind: DiffLineKind::Added,
                old: None,
                new: Some(2),
                text: "new();".into(),
            },
        ],
        truncated: false,
    }
}

#[test]
fn changes_panel_toggles_via_keybinding_and_close_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([KeyBinding::new("cmd-shift-j", crate::ToggleChanges, Some("workspace"))]);
        window.draw(cx).clear(cx);
        assert!(window.try_find("changes-panel").is_none(), "panel starts closed");

        window.press("cmd-shift-j", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("changes-panel").visible(), "cmd-shift-j opens the panel");
        assert!(ws.read(cx).changes_panel_open);

        window.click("close-changes", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("changes-panel").is_none(), "close button hides the panel");
        assert!(!ws.read(cx).changes_panel_open);
    });
}

#[test]
fn changes_panel_lists_rows_and_refresh_recollects() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![
                change("src/edited.rs", ChangeStatus::Modified, 3, 1),
                change("src/new.rs", ChangeStatus::Added, 12, 0),
            ];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("change-row", 0usize)).visible(), "first row renders");
        assert!(window.find(("change-row", 1usize)).visible(), "second row renders");

        // Refresh re-runs real git collection — the test cwd is this repo, so
        // the panel keeps working; only the state round-trip is asserted.
        window.click("refresh-changes", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("changes-panel").visible(), "panel stays open after refresh");
    });
}

#[test]
fn change_row_expands_to_show_diff_lines_and_collapses() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, 1, 1)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.try_find(("change-diff", 0usize)).is_none(), "diff starts collapsed");

        // Expand: the row's git diff loads on the background executor — the
        // click returns before it lands.
        window.click(("change-row", 0usize), cx);
        assert!(ws.read(cx).changes[0].diff.is_none(), "diff load is off the click path");
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        // src/edited.rs is absent from this repo's working tree, so the diff
        // is empty and the body renders its placeholder.
        window.draw(cx).clear(cx);
        assert!(window.find(("change-diff", 0usize)).visible(), "click expands the diff body");
        assert!(ws.read(cx).changes[0].diff.is_some(), "diff result is cached on the row");

        window.click(("change-row", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("change-diff", 0usize)).is_none(), "second click collapses");
        assert!(ws.read(cx).changes[0].diff.is_none(), "collapse drops the cached diff");
    });
}

/// Opening the panel must not run git collection on the UI thread: the list
/// stays empty until the background task lands, then shows the repo's rows.
/// Skips its assertions when `git` is unavailable.
#[test]
fn opening_panel_collects_off_the_ui_thread() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let dir = std::env::temp_dir().join(format!("rixlcode-changes-collect-{}", std::process::id()));
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
        return;
    }
    std::fs::write(dir.join("tracked.txt"), "one\n").unwrap();
    run(&["add", "tracked.txt"]);
    run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
    std::fs::write(dir.join("tracked.txt"), "one\ntwo\n").unwrap();

    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.toggle_changes_panel(cx);
        });
        assert!(ws.read(cx).changes_panel_open, "panel opened");
        assert!(ws.read(cx).changes.is_empty(), "collection did not run on the UI thread");
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let changes = &ws.read(cx).changes;
        assert_eq!(changes.len(), 1, "background collection published the row");
        assert_eq!(changes[0].path, "tracked.txt");
        assert_eq!(changes[0].status, ChangeStatus::Modified);
        assert_eq!(changes[0].added, 1);
        assert!(window.find(("change-row", 0usize)).visible(), "row renders after collection");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn expanded_row_renders_parsed_diff_lines() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            let mut c = change("src/edited.rs", ChangeStatus::Modified, 1, 1);
            c.diff = Some(sample_diff());
            this.changes = vec![c];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("change-diff", 0usize)).visible(), "expanded diff body renders");
        for line in 0..4usize {
            assert!(window.find(("diff-line", line)).visible(), "diff line {line} renders");
        }
    });
}

/// A refresh result stamped with an older generation must not publish: a
/// newer refresh was requested while it ran, so landing it would revert the
/// panel to a stale snapshot.
#[test]
fn stale_refresh_result_is_discarded() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("old.rs", ChangeStatus::Modified, 1, 0)];
            let stale_gen = this.changes_generation;
            this.changes_generation += 1; // a newer refresh was requested
            this.land_changes(
                stale_gen,
                crate::changes::ChangesSnapshot {
                    changes: vec![change("stale.rs", ChangeStatus::Added, 5, 0)],
                    branch: None,
                    commits: vec![],
                },
                cx,
            );
            assert_eq!(this.changes[0].path, "old.rs", "stale collection did not publish");
            this.land_changes(
                this.changes_generation,
                crate::changes::ChangesSnapshot {
                    changes: vec![change("fresh.rs", ChangeStatus::Added, 5, 0)],
                    branch: None,
                    commits: vec![],
                },
                cx,
            );
            assert_eq!(this.changes[0].path, "fresh.rs", "current collection publishes");
        });
    });
}

/// Clicking a row while its diff load is still running collapses it: the
/// pending token clears and the late result is discarded instead of opening
/// the row.
#[test]
fn second_click_during_load_collapses_and_discards() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("src/edited.rs", ChangeStatus::Modified, 1, 1)];
            this.changes_panel_open = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);

        window.click(("change-row", 0usize), cx);
        assert_ne!(ws.read(cx).changes[0].diff_load, 0, "first click started a load");
        window.click(("change-row", 0usize), cx);
        assert_eq!(ws.read(cx).changes[0].diff_load, 0, "second click cancelled the load");
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("change-diff", 0usize)).is_none(), "late result did not reopen the row");
        assert!(ws.read(cx).changes[0].diff.is_none());
        assert_eq!(ws.read(cx).changes[0].diff_load, 0);
    });
}

/// A diff load that outlives a refresh must not attach to the refreshed row:
/// the generation it was issued under no longer matches. A collapse+re-expand
/// under the same generation likewise discards the older load — its token is
/// stale even though the row is pending again.
#[test]
fn stale_diff_results_are_discarded() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![change("f.rs", ChangeStatus::Modified, 1, 1)];
            let generation = this.changes_generation;
            this.toggle_change_diff(0, cx);
            let token = this.changes[0].diff_load;

            // Refresh requested under the load → the diff's generation is stale.
            this.changes_generation += 1;
            this.land_change_diff((generation, token), Some(sample_diff()), cx);
            assert!(this.changes[0].diff.is_none(), "pre-refresh diff did not attach");

            // Collapse + re-expand → the older load's token no longer matches.
            this.changes_generation = generation;
            this.toggle_change_diff(0, cx);
            this.toggle_change_diff(0, cx);
            let newer = this.changes[0].diff_load;
            assert_ne!(newer, token, "re-expand stamped a fresh token");
            this.land_change_diff((generation, token), Some(sample_diff()), cx);
            assert!(this.changes[0].diff.is_none(), "stale-token diff did not attach");
            this.land_change_diff((generation, newer), Some(sample_diff()), cx);
            assert!(this.changes[0].diff.is_some(), "current-token diff attaches");
        });
    });
}
