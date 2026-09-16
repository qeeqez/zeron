//! Line-level diff body for an expanded Changes-panel row — Codex-style
//! tinted added/removed rows with old/new line-number gutters and `@@` hunk
//! headers — plus the diff-review UI: numbered lines are clickable and open
//! an inline comment editor, commented lines carry a marker, and
//! `review_banner` renders the pending review above the file list.
//! Rendering only; parsing lives in `crate::changes_diff`, comment state in
//! `crate::review`.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes_diff::diff_highlight::MarkedDiff;
use crate::changes_diff::{DiffLineKind, DiffMode};
use crate::git::FileChange;
use crate::model::{ReviewComment, ReviewTarget};
use crate::workspace::Workspace;

/// The expanded diff under `change`'s file row. `next_line` is a running
/// counter across the panel so every unified row gets a unique test id;
/// split rows key their cells by diff-line index instead. Right-click
/// anywhere in the body opens the file menu.
pub fn render_diff(file_ix: usize, change: &FileChange, next_line: &mut usize, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let diff = change.diff.as_ref().expect("render_diff needs an expanded diff");
    // Copy the theme fields up front — `cx.theme()` borrows `*cx` and the
    // per-line builders below need `&mut cx` for their click listeners.
    let (border, muted_fg, mono) = {
        let theme = cx.theme();
        (theme.border, theme.muted_foreground, theme.mono_font_family.clone())
    };
    let mut body = div()
        .id(("change-diff", file_ix))
        .test_support()
        .flex()
        .flex_col()
        .w_full()
        .overflow_x_scroll()
        .border_t_1()
        .border_color(border)
        .py_1()
        .text_xs()
        .font_family(mono);
    if diff.lines.is_empty() {
        body = body.child(div().px_2().py_1().text_color(muted_fg).child("No textual diff — binary or unchanged file"));
    } else {
        let rows = match ws.diff_mode {
            DiffMode::Unified => unified_rows(file_ix, diff, next_line, ws, cx),
            DiffMode::Split => crate::views::diff_split::render_rows(file_ix, diff, ws, cx),
        };
        body = body.children(rows);
        if diff.truncated {
            body = body.child(div().px_2().py_1().text_color(muted_fg).child("… diff truncated"));
        }
    }
    body.context_menu({
        let ws = cx.entity();
        let path = change.path.clone();
        move |menu, window, cx| crate::open_in::file_menu(&ws, &path, menu, window, cx)
    })
    .into_any_element()
}

/// The unified layout's rows: one numbered row per diff line, with the
/// comment editor mounted under its anchor's row.
fn unified_rows(
    file_ix: usize, diff: &crate::changes_diff::FileDiff, next_line: &mut usize, ws: &Workspace, cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let marked = MarkedDiff::new(diff, !ws.git.ignore_ws);
    let mut rows = Vec::with_capacity(diff.lines.len());
    for line_ix in 0..diff.lines.len() {
        let id = *next_line;
        *next_line += 1;
        let target = ReviewTarget { file_ix, line_ix };
        rows.push(diff_line(id, target, &marked, ws, cx));
        if ws.review.target == Some(target) {
            rows.push(comment_editor(target, ws, cx));
        }
    }
    rows
}

/// One numbered diff row: `old new │ sign text`, tinted by line kind. Rows
/// with a line number are clickable — a click anchors the comment editor —
/// and a row whose line already has a comment shows it after the code.
/// Paired removed/added lines carry their changed range as a stronger wash.
pub(crate) fn diff_line(id: usize, target: ReviewTarget, marked: &MarkedDiff, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let line = &marked.diff.lines[target.line_ix];
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
    let commentable = line.new.or(line.old).is_some();
    let mut row = div()
        .id(("diff-line", id))
        .test_support()
        .flex()
        .items_center()
        .w_auto()
        .min_w_full()
        .whitespace_nowrap()
        .child(gutter(line.old))
        .child(gutter(line.new))
        .child(div().w(px(14.)).flex_shrink_0().text_center().text_color(fg).child(sign))
        .child(div().text_color(fg).child(marked.code_text(target.line_ix, fg)));
    if let Some(tint) = tint {
        row = row.bg(tint);
    }
    if commentable {
        row = row
            .cursor_pointer()
            .hover(|d| d.bg(theme.muted.opacity(0.4)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_review_comment(target.file_ix, target.line_ix, window, cx);
            }));
        if let Some(ix) = ws.review_comment_at(target) {
            row = row.child(comment_chip(&ws.review.comments[ix], cx));
        }
    }
    row.into_any_element()
}

/// The inline marker a commented diff line carries: a speech-bubble icon
/// plus the comment text, appended after the code in a unified row or
/// inside the owning cell in a split row.
pub(crate) fn comment_chip(comment: &ReviewComment, cx: &mut Context<Workspace>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .pl_2()
        .text_color(cx.theme().info)
        .child(IconName::MessageSquare)
        .child(comment.text.clone())
}

/// The inline comment editor under its anchored diff row: the `path:line`
/// label, the shared review input, a commit ✓ and a cancel ✕.
pub(crate) fn comment_editor(target: ReviewTarget, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let label = crate::changes_diff::review_anchor(&ws.changes, target)
        .map(|a| format!("{}:{}", a.path, a.line))
        .unwrap_or_default();
    div()
        .id("review-editor")
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.muted.opacity(0.3))
        .child(div().flex_shrink_0().text_color(theme.info).child(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&ws.review.input).id("review-input").appearance(true).w_full()),
        )
        .child(
            div()
                .id("review-commit")
                .test_support()
                .cursor_pointer()
                .text_color(theme.success)
                .child(IconName::Check)
                .on_click(cx.listener(|this, _, _, cx| this.commit_review_comment(cx))),
        )
        .child(
            div()
                .id("review-cancel")
                .test_support()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .child(IconName::X)
                .on_click(cx.listener(|this, _, _, cx| this.cancel_review_comment(cx))),
        )
        .into_any_element()
}

/// The pending-review strip between the panel header and the file list:
/// a count, one removable row per comment (click reopens the editor on its
/// diff line), and the Send button that ships the review to the agent.
pub fn review_banner(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    // Copy theme fields up front — `cx.theme()` borrows `*cx` and the row
    // builder below needs `&mut cx` for its click listeners.
    let (border, info, accent, accent_fg) = {
        let theme = cx.theme();
        (theme.border, theme.info, theme.accent, theme.accent_foreground)
    };
    let rows: Vec<AnyElement> = ws.review.comments.iter().enumerate().map(|(ix, c)| review_banner_row(ix, c, ws, cx)).collect();
    let n = ws.review.comments.len();
    div()
        .id("review-banner")
        .test_support()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(border)
        .px_3()
        .py_2()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(div().text_color(info).child(IconName::MessageSquareDiff))
                .child(format!("{n} review comment{}", if n == 1 { "" } else { "s" }))
                .child(div().flex_1())
                .child(
                    div()
                        .id("send-review")
                        .test_support()
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded_md()
                        .px_2()
                        .py_1()
                        .bg(accent)
                        .text_color(accent_fg)
                        .child(IconName::Send)
                        .child("Send review")
                        .on_click(cx.listener(|this, _, window, cx| this.send_review(window, cx))),
                ),
        )
        .children(rows)
        .into_any_element()
}

/// One comment row in the banner: `path:line` + text, click to edit on its
/// diff line (only while that diff is expanded), ✕ to remove.
fn review_banner_row(ix: usize, comment: &ReviewComment, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let target = ws.review_target_for(comment);
    let mut row = div().id(("review-comment", ix)).test_support().flex().items_center().gap_2().text_xs().child(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(format!("{}:{} — {}", comment.path, comment.line, comment.text)),
    );
    if let Some(target) = target {
        row = row
            .cursor_pointer()
            .hover(|d| d.bg(theme.muted.opacity(0.4)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_review_comment(target.file_ix, target.line_ix, window, cx);
            }));
    }
    row.child(
        div()
            .id(("review-remove", ix))
            .test_support()
            .cursor_pointer()
            .flex_shrink_0()
            .text_color(theme.muted_foreground)
            .child(IconName::X)
            .on_click(cx.listener(move |this, _, _, cx| {
                // Keep the click off the row — removing must not reopen the editor.
                cx.stop_propagation();
                this.remove_review_comment(ix, cx);
            })),
    )
    .into_any_element()
}
