//! The unified diff layout's rows — `unified_rows` walks a `FileDiff` into
//! numbered rows and `diff_line` renders one. Split from `diff.rs` for the
//! SLOC cap; re-exported there so `diff_split` keeps using
//! `crate::views::diff::diff_line` / `DiffRow`. Hunk headers of a modified
//! file carry a Stage/Unstage button — git can't partially apply additions,
//! deletions, or renames, so only `Modified` rows offer it, and only for
//! hunks with a recorded patch range.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes::GitOp;
use crate::changes_diff::DiffLineKind;
use crate::changes_diff::diff_highlight::MarkedDiff;
use crate::git::{ChangeStatus, FileChange};
use crate::model::ReviewTarget;
use crate::workspace::Workspace;

/// What a hunk header's Stage/Unstage button applies: the file's path plus
/// the single-hunk patch carved from the rendered diff. `staged` picks the
/// label and the `GitOp` — staged rows show the HEAD→index diff and reverse
/// it, unstaged rows show the index→worktree diff and apply it.
#[derive(Clone)]
pub(crate) struct HunkTarget {
    path: String,
    patch: String,
    staged: bool,
}

/// `diff_line`'s row identity: the unique test id, the review anchor, and —
/// unified view only — the hunk a header row can stage.
pub(crate) struct DiffRow {
    pub id: usize,
    pub target: ReviewTarget,
    pub hunk: Option<HunkTarget>,
}

/// The unified layout's rows: one numbered row per diff line, with the
/// comment editor mounted under its anchor's row.
pub(crate) fn unified_rows(
    file_ix: usize, change: &FileChange, next_line: &mut usize, ws: &Workspace, cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let diff = change.diff.as_ref().expect("unified_rows needs an expanded diff");
    let marked = MarkedDiff::new(diff, !ws.git.ignore_ws);
    let stageable = change.status == ChangeStatus::Modified;
    let mut hunk_ix = 0usize;
    let mut rows = Vec::with_capacity(diff.lines.len());
    for line_ix in 0..diff.lines.len() {
        let id = *next_line;
        *next_line += 1;
        let target = ReviewTarget { file_ix, line_ix };
        let hunk = if stageable && diff.lines[line_ix].kind == DiffLineKind::Hunk {
            let ix = hunk_ix;
            hunk_ix += 1;
            diff.hunk_patch(ix)
                .map(|patch| HunkTarget { path: change.path.clone(), patch, staged: change.staged })
        } else {
            None
        };
        rows.push(diff_line(DiffRow { id, target, hunk }, &marked, ws, cx));
        if ws.review.target == Some(target) {
            rows.push(crate::views::diff::comment_editor(target, ws, cx));
        }
    }
    rows
}

/// One numbered diff row: `+ old new │ sign text`, tinted by line kind.
/// Every numbered row is clickable — a ⌘-click opens the file at that
/// line — and rows with a new-side number (added or context, never
/// removed) are commentable: hovering reveals a `+` affordance and a
/// click anchors the comment editor. A row whose line already has a
/// comment shows it after the code.
/// Paired removed/added lines carry their changed range as a stronger wash.
/// A hunk header row with a `HunkTarget` right-aligns a Stage/Unstage ghost
/// button that applies just that hunk to the index.
pub(crate) fn diff_line(row: DiffRow, marked: &MarkedDiff, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let line = &marked.diff.lines[row.target.line_ix];
    let theme = cx.theme();
    let (tint, fg, sign) = match line.kind {
        DiffLineKind::Added => (Some(theme.success.opacity(0.12)), theme.success, "+"),
        DiffLineKind::Removed => (Some(theme.danger.opacity(0.12)), theme.danger, "-"),
        DiffLineKind::Hunk => (Some(theme.info.opacity(0.08)), theme.info, " "),
        DiffLineKind::Context => (None, theme.foreground, " "),
    };
    let gutter = |n: Option<u32>| {
        div()
            .w(px(30.))
            .flex_shrink_0()
            .text_right()
            .text_color(theme.muted_foreground)
            .child(n.map(|n| n.to_string()).unwrap_or_default())
    };
    let clickable = line.new.or(line.old).is_some();
    let commentable = line.new.is_some();
    let group = || SharedString::from(format!("diff-row-{}", row.id));
    let mut code = div().text_color(fg).child(marked.code_text(row.target.line_ix, fg));
    if row.hunk.is_some() {
        // Grow the code cell so the hunk button pins to the row's right edge.
        code = code.flex_1().min_w_0();
    }
    let mut row_div = div()
        .id(("diff-line", row.id))
        .test_support()
        .flex()
        .items_center()
        .w_auto()
        .min_w_full()
        .whitespace_nowrap()
        .child(
            div()
                .w(px(16.))
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .when(commentable, |d| d.child(div().invisible().group_hover(group(), |s| s.visible()).child(IconName::Plus))),
        )
        .child(gutter(line.old))
        .child(gutter(line.new))
        .child(div().w(px(14.)).flex_shrink_0().text_center().text_color(fg).child(sign))
        .child(code);
    if let Some(tint) = tint {
        row_div = row_div.bg(tint);
    }
    if let Some(hunk) = row.hunk {
        row_div = row_div.child(
            div().flex_shrink_0().pl_2().pr_1().child(
                Button::new(("hunk-stage", row.id))
                    .ghost()
                    .xsmall()
                    .label(if hunk.staged { "Unstage" } else { "Stage" })
                    .tooltip(if hunk.staged { "Unstage this hunk" } else { "Stage this hunk" })
                    .disabled(ws.git.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let op = if hunk.staged {
                            GitOp::UnstageHunk { path: hunk.path.clone(), patch: hunk.patch.clone() }
                        } else {
                            GitOp::StageHunk { path: hunk.path.clone(), patch: hunk.patch.clone() }
                        };
                        this.run_git_op(op, cx);
                    })),
            ),
        );
    }
    if clickable {
        row_div = row_div
            .cursor_pointer()
            .hover(|d| d.bg(theme.muted.opacity(0.4)))
            .tooltip(move |window, cx| {
                let tip = if commentable {
                    "Click to comment · ⌘-click opens in editor"
                } else {
                    "⌘-click opens in editor"
                };
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .on_click(cx.listener(move |this, event, window, cx| {
                this.click_diff_line(row.target, event, window, cx);
            }));
        if let Some(ix) = ws.review_comment_at(row.target) {
            row_div = row_div.child(crate::views::diff::comment_chip(&ws.review.comments[ix], cx));
        }
    }
    if commentable {
        row_div = row_div.group(group());
    }
    row_div.into_any_element()
}
