//! The file-inspect overlay: a centered modal over a dimmed backdrop showing
//! either a file's `git log --follow` history (one commit row each, click to
//! expand that commit's diff for the file) or its `git blame` (a monospace
//! `sha author line` list — click a row to copy the sha). Opened from the
//! shared file menu's "File History"/"Blame" items; mounted by
//! `Workspace::render` while `Workspace::file_inspect` is set. Esc (via
//! `escape_key` in `root`), the header ✕, or a backdrop click closes it.
//! State and ops live in `file_inspect_ops.rs`, declared below via `#[path]`
//! so `main.rs` stays under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{BlameLine, Commit, CommitDiff};
use crate::workspace::Workspace;

#[path = "../file_inspect_ops.rs"]
mod ops;
pub(crate) use ops::FileInspect;

/// The overlay root: full-window backdrop + centered panel. The backdrop's
/// hitbox covers the window, so a press anywhere the panel doesn't occlude
/// lands on it and closes the panel.
pub(crate) fn file_inspect_overlay(this: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let backdrop = div()
        .id("file-inspect-backdrop")
        .test_support()
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_file_inspect(cx)));
    div()
        .id("file-inspect-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .child(backdrop)
        .child(div().absolute().inset_0().flex().items_center().justify_center().child(panel(this, cx)))
}

/// The centered card: header (icon, title, file path, close) above a
/// scrollable column of commit or blame rows.
fn panel(this: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let theme = cx.theme();
    div()
        .occlude()
        .w(px(760.))
        .h(px(480.))
        .flex()
        .flex_col()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded_lg()
        .shadow_lg()
        .child(header(this, cx))
        .child(body(this, cx))
}

fn header(this: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let theme = cx.theme();
    let (icon, title, path) = match &this.file_inspect {
        Some(FileInspect::History { path, .. }) => (IconName::GitCommitHorizontal, "File History", path.clone()),
        Some(FileInspect::Blame { path, .. }) => (IconName::UserSearch, "Blame", path.clone()),
        None => (IconName::File, "", String::new()),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(icon)
                .child(title),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(path),
        )
        .child(
            div()
                .id("file-inspect-close")
                .test_support()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(IconName::X)
                .on_click(cx.listener(|this, _, _, cx| this.close_file_inspect(cx))),
        )
}

/// The scrollable body — loading text, an error note, or the fetched rows.
fn body(this: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let status = |text: String| -> AnyElement {
        div()
            .id("file-inspect-status")
            .test_support()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(muted)
            .child(text)
            .into_any_element()
    };
    match &this.file_inspect {
        Some(FileInspect::History { result: None, .. }) | Some(FileInspect::Blame { result: None, .. }) => status("Loading…".to_string()),
        Some(FileInspect::History { result: Some(Err(e)), .. }) | Some(FileInspect::Blame { result: Some(Err(e)), .. }) => {
            status(e.clone())
        },
        Some(FileInspect::History { result: Some(Ok(commits)), .. }) if commits.is_empty() => {
            status("No commits touch this file".to_string())
        },
        Some(FileInspect::History { result: Some(Ok(commits)), .. }) => history_rows(commits, cx),
        Some(FileInspect::Blame { result: Some(Ok(lines)), .. }) => blame_rows(lines, cx),
        None => status(String::new()),
    }
}

/// One row per commit touching the file — `abc1234 subject … author · 2h
/// ago`, same shape as the Changes panel's "Recent commits" rows. Click
/// expands the commit's diff for this file.
fn history_rows(commits: &[Commit], cx: &mut Context<Workspace>) -> AnyElement {
    let mut list = v_flex()
        .id("file-history-rows")
        .test_support()
        .flex_1()
        .min_h_0()
        .p_2()
        .gap_0p5()
        .overflow_y_scroll();
    for (ix, commit) in commits.iter().enumerate() {
        let mut entry = v_flex().child(history_row(ix, commit, cx));
        if let Some(diff) = &commit.diff {
            entry = entry.child(history_diff_body(ix, diff, cx));
        }
        list = list.child(entry);
    }
    list.into_any_element()
}

/// `abc1234 subject … author · 2h ago` — click toggles the inline diff.
fn history_row(ix: usize, commit: &Commit, cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    let theme = cx.theme();
    let (muted_fg, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
    let expanded = commit.diff.is_some();
    div()
        .id(("file-history-row", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .rounded_md()
        .text_xs()
        .cursor_pointer()
        .hover(|d| d.bg(cx.theme().muted))
        .child(
            div()
                .flex_shrink_0()
                .text_color(muted_fg)
                .child(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }),
        )
        .child(div().flex_shrink_0().font_family(mono).text_color(cx.theme().info).child(commit.hash.clone()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(commit.subject.clone()),
        )
        .child(div().flex_shrink_0().text_color(muted_fg).child(format!("{} · {}", commit.author, commit.rel_time)))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_history_diff(ix, cx)))
}

/// The expanded `git show <sha> -- <path>` body under a history row — the
/// Changes panel's commit-diff line renderer under an overlay-local id.
fn history_diff_body(ix: usize, diff: &CommitDiff, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (border, muted_fg, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
    let mut body = v_flex()
        .id(("file-history-diff", ix))
        .test_support()
        .w_full()
        .overflow_x_scroll()
        .border_t_1()
        .border_color(border)
        .py_1()
        .text_xs()
        .font_family(mono);
    if diff.files.is_empty() {
        body = body.child(div().px_2().py_1().text_color(muted_fg).child("No textual diff for this file"));
    } else {
        for file in &diff.files {
            for line in &file.diff.lines {
                body = body.child(super::changes_commits::commit_diff_line(line, cx));
            }
        }
        if diff.truncated {
            body = body.child(div().px_2().py_1().text_color(muted_fg).child("… diff truncated"));
        }
    }
    body.into_any_element()
}

/// The blame list — monospace `sha author line` rows; clicking a row copies
/// the commit hash. `aria_label` carries the row so headless tests can
/// assert content.
fn blame_rows(lines: &[BlameLine], cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (muted_fg, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
    let mut list = div()
        .id("file-blame-rows")
        .test_support()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .p_2()
        .overflow_y_scroll()
        .font_family(mono)
        .text_xs();
    for (ix, line) in lines.iter().enumerate() {
        list = list.child(blame_row(ix, line, muted_fg, cx));
    }
    list.into_any_element()
}

/// One blame row: short sha, author, line number, then the line's text.
/// Click copies the full commit hash.
fn blame_row(ix: usize, line: &BlameLine, muted_fg: Hsla, cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    let sha = line.sha.clone();
    let short: String = line.sha.chars().take(8).collect();
    div()
        .id(("file-blame-row", ix))
        .test_support()
        .aria_label(format!("{short} {} {}", line.line_no, line.text))
        .flex()
        .items_baseline()
        .gap_2()
        .px_2()
        .py_0p5()
        .rounded_md()
        .cursor_pointer()
        .whitespace_nowrap()
        .hover(|d| d.bg(cx.theme().muted))
        .child(div().flex_shrink_0().text_color(cx.theme().info).child(short))
        .child(
            div()
                .flex_shrink_0()
                .w(px(96.))
                .overflow_hidden()
                .text_ellipsis()
                .text_color(muted_fg)
                .child(line.author.clone()),
        )
        .child(div().flex_shrink_0().w(px(40.)).text_right().text_color(muted_fg).child(line.line_no.to_string()))
        .child(div().child(line.text.clone()))
        .on_click(cx.listener(move |this, _, _, cx| this.copy_blame_sha(&sha, cx)))
}
