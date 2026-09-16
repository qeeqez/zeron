//! Split (side-by-side) rendering for an expanded Changes-panel diff: old
//! lines on the left, new on the right, aligned by `crate::changes_diff::
//! split_rows`. Row pairing lives in `changes_diff`; this file only draws.
//! Cells keep the unified view's review affordance — a numbered cell is
//! clickable and anchors the comment editor to its `DiffLine` index, so
//! `ReviewTarget`s resolve identically in both modes.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes_diff::diff_highlight::MarkedDiff;
use crate::changes_diff::{DiffLineKind, FileDiff, SplitRow};
use crate::model::ReviewTarget;
use crate::workspace::Workspace;

/// The split layout's rows for one file: paired old|new cells plus
/// full-width hunk/marker rows, with the comment editor mounted under the
/// row its anchor lives in.
pub(crate) fn render_rows(file_ix: usize, diff: &FileDiff, ws: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let marked = MarkedDiff::new(diff, !ws.git.ignore_ws);
    let mut rows = Vec::with_capacity(diff.lines.len());
    for row in crate::changes_diff::split_rows(diff) {
        rows.push(render_row(file_ix, row, &marked, ws, cx));
        if let Some(t) = ws.review.target.filter(|t| t.file_ix == file_ix && row.contains(t.line_ix)) {
            rows.push(crate::views::diff::comment_editor(t, ws, cx));
        }
    }
    rows
}

/// One split row inside `("change-diff", file_ix)`: a full-width line via
/// the unified renderer, or an old|new cell pair. Cell ids are
/// `("diff-cell-old"|"diff-cell-new", line_ix)` — keyed by diff-line index,
/// unique within the file's diff.
fn render_row(file_ix: usize, row: SplitRow, marked: &MarkedDiff, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let target = |ix: usize| ReviewTarget { file_ix, line_ix: ix };
    match row {
        SplitRow::Wide(ix) => crate::views::diff::diff_line(ix, target(ix), marked, ws, cx),
        SplitRow::Pair { old, new } => div()
            .flex()
            .items_stretch()
            .w_full()
            .child(cell(old.map(target), Side::Old, marked, ws, cx))
            .child(cell(new.map(target), Side::New, marked, ws, cx))
            .into_any_element(),
    }
}

/// Which column a cell sits in — picks the line's gutter number and tint.
#[derive(Clone, Copy)]
enum Side {
    Old,
    New,
}

/// One half of a paired row: a 30px gutter plus the line text, tinted like
/// the unified view (danger on the old side, success on the new). An empty
/// cell — the missing half of an unpaired removal or addition — gets a
/// muted fill. Commented lines carry the same chip as unified rows.
fn cell(target: Option<ReviewTarget>, side: Side, marked: &MarkedDiff, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    // Copy theme fields up front — `cx.theme()` borrows `*cx` and the
    // listeners below need `&mut cx`.
    let (border, muted, muted_fg, success, danger, fg) = {
        let t = cx.theme();
        (t.border, t.muted, t.muted_foreground, t.success, t.danger, t.foreground)
    };
    // The old column's right edge is the divider between the two sides.
    let divider = |d: Div| d.border_r_1().border_color(border);
    let Some(target) = target else {
        return div()
            .flex()
            .flex_1()
            .min_w_0()
            .when(matches!(side, Side::Old), divider)
            .bg(muted.opacity(0.15))
            .into_any_element();
    };
    let line = &marked.diff.lines[target.line_ix];
    let (tint, text_fg) = match line.kind {
        DiffLineKind::Added => (Some(success.opacity(0.12)), success),
        DiffLineKind::Removed => (Some(danger.opacity(0.12)), danger),
        _ => (None, fg),
    };
    let number = match side {
        Side::Old => line.old,
        Side::New => line.new,
    };
    let mut cell = div()
        .id(match side {
            Side::Old => ("diff-cell-old", target.line_ix),
            Side::New => ("diff-cell-new", target.line_ix),
        })
        .test_support()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .whitespace_nowrap()
        .overflow_hidden()
        .when(matches!(side, Side::Old), |d| d.border_r_1().border_color(border))
        .when_some(tint, |d, t| d.bg(t))
        .child(
            div()
                .w(px(30.))
                .flex_shrink_0()
                .text_right()
                .pr_1()
                .text_color(muted_fg)
                .child(number.map(|n| n.to_string()).unwrap_or_default()),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_color(text_fg)
                .child(marked.code_text(target.line_ix, text_fg)),
        );
    if line.new.or(line.old).is_some() {
        cell = cell
            .cursor_pointer()
            .hover(|d| d.bg(muted.opacity(0.4)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_review_comment(target.file_ix, target.line_ix, window, cx);
            }));
        if let Some(cix) = ws.review_comment_at(target) {
            cell = cell.child(crate::views::diff::comment_chip(&ws.review.comments[cix], cx));
        }
    }
    cell.into_any_element()
}
