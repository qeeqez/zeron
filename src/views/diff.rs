//! Line-level diff body for an expanded Changes-panel row — Codex-style
//! tinted added/removed rows with old/new line-number gutters and `@@` hunk
//! headers. Rendering only; parsing lives in `crate::changes_diff`.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes_diff::{DiffLine, DiffLineKind, FileDiff};

/// The expanded diff under file row `file_ix`. `next_line` is a running
/// counter across the panel so every row gets a unique test id.
pub fn render_diff(file_ix: usize, diff: &FileDiff, next_line: &mut usize, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let mut body = div()
        .id(("change-diff", file_ix))
        .test_support()
        .flex()
        .flex_col()
        .w_full()
        .overflow_x_scroll()
        .border_t_1()
        .border_color(theme.border)
        .py_1()
        .text_xs()
        .font_family(theme.mono_font_family.clone());
    if diff.lines.is_empty() {
        body = body.child(
            div()
                .px_2()
                .py_1()
                .text_color(theme.muted_foreground)
                .child("No textual diff — binary or unchanged file"),
        );
    } else {
        for line in &diff.lines {
            let id = *next_line;
            *next_line += 1;
            body = body.child(diff_line(id, line, cx));
        }
        if diff.truncated {
            body = body.child(div().px_2().py_1().text_color(theme.muted_foreground).child("… diff truncated"));
        }
    }
    body.into_any_element()
}

/// One numbered diff row: `old new │ sign text`, tinted by line kind.
fn diff_line(id: usize, line: &DiffLine, cx: &App) -> AnyElement {
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
        .child(div().text_color(fg).child(line.text.clone()));
    if let Some(tint) = tint {
        row = row.bg(tint);
    }
    row.into_any_element()
}
