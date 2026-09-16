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

use crate::changes_diff::DiffMode;
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
            DiffMode::Unified => unified_rows(file_ix, change, next_line, ws, cx),
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
        let staged = change.staged;
        move |menu, window, cx| {
            crate::open_in::file_menu(&ws, &path, menu, window, cx).item(crate::open_in::copy_diff_item(&ws, &path, staged))
        }
    })
    .into_any_element()
}
/// The unified layout's rows — `unified_rows`, `diff_line`, and the hunk
/// Stage/Unstage button — split into `diff_rows.rs` for the SLOC cap;
/// re-exported so `diff_split` keeps using `crate::views::diff::diff_line`.
#[path = "diff_rows.rs"]
pub(crate) mod rows;
use rows::unified_rows;
pub(crate) use rows::{DiffRow, diff_line};

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
/// diff line), and the Send button that stages the review in the composer.
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
                        .child(format!("Send {n} comment{}", if n == 1 { "" } else { "s" }))
                        .on_click(cx.listener(|this, _, window, cx| this.draft_review(window, cx))),
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
