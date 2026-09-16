//! Comment lifecycle: pending review comments are keyed to the file's
//! current diff — a refresh that removes the file or a reload that drops
//! the line drops its comments; a file whose diff isn't loaded keeps them
//! (nothing proved the line is gone).

use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use super::{change, mount, sample_diff, seed_diff};
use crate::changes::ChangesSnapshot;
use crate::git::ChangeStatus;
use crate::workspace::Workspace;

/// Commit `text` as a comment on diff line `line_ix` of file 0.
fn comment_on(workspace: &Entity<Workspace>, line_ix: usize, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_review_comment(0, line_ix, window, cx);
            ws.review.input.update(cx, |s, cx| s.set_value(text, window, cx));
            ws.commit_review_comment(cx);
        });
    });
}

/// A landed refresh carrying `changes` — the rest of the snapshot is empty.
fn land(workspace: &Entity<Workspace>, changes: Vec<crate::git::FileChange>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            let generation = ws.changes_generation;
            ws.land_changes(
                generation,
                ChangesSnapshot {
                    changes,
                    branch: None,
                    commits: vec![],
                    stashes: vec![],
                    conflicts: vec![],
                    pr: None,
                },
                cx,
            );
        });
    });
}

#[gpui_kit::test]
fn refresh_without_the_file_drops_its_comments(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, cx);
    comment_on(&ws, 1, "fix this", cx);
    assert_eq!(ws.read_with(cx, |ws, _| ws.review.comments.len()), 1);

    land(&ws, vec![], cx);
    assert!(ws.read_with(cx, |ws, _| ws.review.comments.is_empty()), "a refresh that removes the file drops its comments");
}

#[gpui_kit::test]
fn refresh_keeps_comments_while_the_diff_is_unloaded(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, cx);
    comment_on(&ws, 1, "fix this", cx);

    // Refresh lands the same file with its diff unloaded — nothing proved
    // the line is gone, so the comment survives.
    land(&ws, vec![change("src/edited.rs", ChangeStatus::Modified, 1, 1)], cx);
    assert_eq!(ws.read_with(cx, |ws, _| ws.review.comments.len()), 1, "an unloaded diff can't drop a comment");
}

#[gpui_kit::test]
fn reload_without_the_line_drops_its_comment(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, cx);
    comment_on(&ws, 1, "fix this", cx);
    comment_on(&ws, 3, "rename this", cx);
    land(&ws, vec![change("src/edited.rs", ChangeStatus::Modified, 1, 1)], cx);

    // The file's diff reloads without the context line — its comment drops,
    // the added line's survives.
    cx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.changes[0].diff_load = 7;
            let generation = ws.changes_generation;
            let mut diff = sample_diff();
            diff.lines.remove(1);
            ws.land_change_diff((generation, 7), Some(diff), cx);
        });
    });
    let comments = &ws.read_with(cx, |ws, _| ws.review.comments.clone());
    assert_eq!(comments.len(), 1, "the dropped line's comment is pruned");
    assert_eq!(comments[0].text, "rename this");
}
