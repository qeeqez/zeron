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
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

fn change(path: &str, status: ChangeStatus, added: u32, deleted: u32) -> FileChange {
    FileChange { path: path.into(), status, added, deleted, diff: None }
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

        // Expand: the row shells out to git for src/edited.rs — absent from
        // this repo's working tree, so the diff is empty and the body still
        // renders its placeholder.
        window.click(("change-row", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("change-diff", 0usize)).visible(), "click expands the diff body");
        assert!(ws.read(cx).changes[0].diff.is_some(), "diff result is cached on the row");

        window.click(("change-row", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("change-diff", 0usize)).is_none(), "second click collapses");
        assert!(ws.read(cx).changes[0].diff.is_none(), "collapse drops the cached diff");
    });
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
