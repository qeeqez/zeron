//! Headless tests for the Changes panel's unified|split diff view mode:
//! the header toggle switches the layout, the choice persists to
//! settings.json and seeds the next window, and review anchors resolve in
//! split mode. Same `TestAppContext::single()` pattern as
//! `changes_ui_tests.rs` — narrow imports keep `#[test]` unshadowed.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::changes_diff::{DiffLine, DiffLineKind, DiffMode, FileDiff};
use crate::git::{ChangeStatus, FileChange};
use crate::model::ReviewTarget;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-diffmode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

fn change(path: &str) -> FileChange {
    FileChange {
        path: path.into(),
        source: None,
        status: ChangeStatus::Modified,
        added: 1,
        deleted: 1,
        diff: None,
        staged: false,
        diff_load: 0,
    }
}

/// Hunk + context + removed + added — the added line is `new = 2`, the
/// removed line is `old = 2`, the context line is `1` on both sides.
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

/// Seed one expanded change row with `sample_diff` and open the panel.
fn seed_diff(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            let mut c = change("src/edited.rs");
            c.diff = Some(sample_diff());
            this.changes = vec![c];
            this.changes_panel_open = true;
            cx.notify();
        });
    });
}

#[test]
fn toggle_switches_between_unified_and_split_layouts() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).diff_mode, DiffMode::Unified, "default mode is unified");
        assert!(window.find(("diff-line", 1usize)).visible(), "unified rows render");
        assert!(window.try_find(("diff-cell-new", 3usize)).is_none(), "no split cells in unified mode");

        window.click("diff-mode-split", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).diff_mode, DiffMode::Split);
        // Cells are keyed by diff-line index: the context line spans both
        // sides, the removed line has no new-side cell, the added line has
        // no old-side cell.
        assert!(window.find(("diff-cell-old", 1usize)).visible(), "context line on the old side");
        assert!(window.find(("diff-cell-new", 1usize)).visible(), "context line on the new side");
        assert!(window.find(("diff-cell-old", 2usize)).visible(), "removed line on the old side");
        assert!(window.try_find(("diff-cell-new", 2usize)).is_none(), "removed line has no new-side cell");
        assert!(window.find(("diff-cell-new", 3usize)).visible(), "added line on the new side");
        assert!(window.try_find(("diff-cell-old", 3usize)).is_none(), "added line has no old-side cell");
        assert!(window.try_find(("diff-line", 1usize)).is_none(), "unified numbered rows are gone");

        window.click("diff-mode-unified", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).diff_mode, DiffMode::Unified);
        assert!(window.find(("diff-line", 1usize)).visible(), "unified rows are back");
        assert!(window.try_find(("diff-cell-new", 3usize)).is_none(), "split cells are gone");
    });
}

#[test]
fn diff_mode_persists_to_settings_and_seeds_new_windows() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("diff-mode-split", cx);
    });
    assert_eq!(ws.read_with(cx, |ws, _| ws.diff_mode), DiffMode::Split);
    assert_eq!(crate::persist::load_settings().diff_mode, "split", "toggle writes settings.json");

    // A second window on the same HOME loads the persisted mode.
    let (ws2, _cx2) = cx.add_window_view(Workspace::new);
    assert_eq!(ws2.read_with(cx, |ws, _| ws.diff_mode), DiffMode::Split, "new window loads the persisted mode");
}

#[test]
fn review_anchors_resolve_in_split_mode() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.set_diff_mode(DiffMode::Split, cx));
        window.draw(cx).clear(cx);

        // Clicking the added line's new-side cell anchors the editor to the
        // same ReviewTarget the unified row would use.
        window.click(("diff-cell-new", 3usize), cx);
        assert_eq!(
            ws.read(cx).review.target,
            Some(ReviewTarget { file_ix: 0, line_ix: 3 }),
            "cell click anchors the editor to diff line 3"
        );
        window.draw(cx).clear(cx);
        assert!(window.find("review-editor").visible(), "editor renders under the split row");
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.review.input.update(cx, |s, cx| s.set_value("rename this", window, cx));
            this.commit_review_comment(cx);
        });
        let comments = &ws.read(cx).review.comments;
        assert_eq!(comments.len(), 1);
        assert_eq!((comments[0].line, comments[0].old_side), (2, false), "added line anchors on the new side");

        // The banner's click-to-edit resolves the comment back to its split
        // cell — `review_target_for` is mode-agnostic.
        window.draw(cx).clear(cx);
        window.click(("review-comment", 0usize), cx);
        assert_eq!(
            ws.read(cx).review.target,
            Some(ReviewTarget { file_ix: 0, line_ix: 3 }),
            "banner reopens the editor on the same anchor"
        );
    });
}
