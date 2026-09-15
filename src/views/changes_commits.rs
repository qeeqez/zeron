//! The "Recent commits" section at the foot of the Changes panel's git
//! block: one row per commit (short hash, subject, author, relative time).
//! Clicking a row expands its `git show` patch inline; right-click offers
//! Copy Hash / Show Diff / Revert. Loading and ops live in
//! `crate::changes_commits`; this file only renders `Workspace::git.commits`.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes_diff::{DiffLine, DiffLineKind};
use crate::git::{Commit, CommitDiff};
use crate::workspace::Workspace;

/// The section — mounted by `git_block` only when `git.commits` is non-empty,
/// so unborn HEADs and non-repos never see the header.
pub fn commits_section(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let rows = ws.git.commits.iter().enumerate().map(|(ix, c)| commit_entry(ix, c, cx)).collect::<Vec<_>>();
    v_flex()
        .id("commits-section")
        .test_support()
        .gap_0p5()
        .pt_1()
        .child(
            h_flex()
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(IconName::GitCommitHorizontal)
                .child("Recent commits"),
        )
        .child(v_flex().id("commits-list").max_h(px(240.)).overflow_y_scroll().gap_0p5().children(rows))
        .into_any_element()
}

/// A commit row plus, when expanded, its inline diff.
fn commit_entry(ix: usize, commit: &Commit, cx: &mut Context<Workspace>) -> AnyElement {
    let mut entry = v_flex().child(commit_row(ix, commit, cx));
    if let Some(diff) = &commit.diff {
        entry = entry.child(commit_diff_body(ix, diff, cx));
    }
    entry.into_any_element()
}

/// `abc1234 subject … author · 2h ago` — click toggles the inline diff;
/// right-click opens the commit menu.
fn commit_row(ix: usize, commit: &Commit, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (muted_fg, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
    let expanded = commit.diff.is_some();
    let sha_menu = commit.hash.clone();
    div()
        .id(("commit-row", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
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
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_commit_diff(ix, cx)))
        .context_menu({
            let ws = cx.entity();
            move |menu, _window, cx| commit_menu(&ws, ix, &sha_menu, menu, cx)
        })
        .into_any_element()
}

/// The commit row's right-click menu: copy the hash, expand the diff (same
/// as clicking the row), or revert the commit with `git revert --no-edit`.
fn commit_menu(ws: &Entity<Workspace>, ix: usize, sha: &str, menu: PopupMenu, _cx: &mut Context<PopupMenu>) -> PopupMenu {
    let sha_copy = sha.to_string();
    let ws_diff = ws.clone();
    let ws_revert = ws.clone();
    let sha_revert = sha.to_string();
    menu.item(PopupMenuItem::new("Copy Hash").icon(IconName::Copy).on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(sha_copy.clone()));
    }))
    .item(PopupMenuItem::new("Show Diff").icon(IconName::FileDiff).on_click(move |_, _, cx| {
        ws_diff.update(cx, |this, cx| this.toggle_commit_diff(ix, cx));
    }))
    .item(PopupMenuItem::new("Revert").icon(IconName::Undo2).on_click(move |_, _, cx| {
        ws_revert.update(cx, |this, cx| this.revert_commit(&sha_revert, cx));
    }))
}

/// The expanded `git show` body under a commit row: a path header per file,
/// then its tinted diff lines. Read-only — review comments anchor to
/// working-tree rows only.
fn commit_diff_body(ix: usize, diff: &CommitDiff, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (border, muted_fg, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
    let mut body = v_flex()
        .id(("commit-diff", ix))
        .test_support()
        .w_full()
        .overflow_x_scroll()
        .border_t_1()
        .border_color(border)
        .py_1()
        .text_xs()
        .font_family(mono);
    if diff.files.is_empty() {
        body = body.child(div().px_2().py_1().text_color(muted_fg).child("No textual diff — binary or unchanged files"));
    } else {
        for file in &diff.files {
            body = body.child(
                div()
                    .px_2()
                    .py_0p5()
                    .w_auto()
                    .min_w_full()
                    .whitespace_nowrap()
                    .text_color(muted_fg)
                    .child(file.path.clone()),
            );
            for line in &file.diff.lines {
                body = body.child(commit_diff_line(line, cx));
            }
        }
        if diff.truncated {
            body = body.child(div().px_2().py_1().text_color(muted_fg).child("… diff truncated"));
        }
    }
    body.into_any_element()
}

/// One commit-diff row: `sign text`, tinted by line kind. No line-number
/// gutters — commit lines aren't review-comment anchors.
fn commit_diff_line(line: &DiffLine, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (tint, fg, sign) = match line.kind {
        DiffLineKind::Added => (Some(theme.success.opacity(0.12)), theme.success, "+"),
        DiffLineKind::Removed => (Some(theme.danger.opacity(0.12)), theme.danger, "-"),
        DiffLineKind::Hunk => (Some(theme.info.opacity(0.08)), theme.info, " "),
        DiffLineKind::Context => (None, theme.foreground, " "),
    };
    let mut row = div()
        .flex()
        .items_center()
        .w_auto()
        .min_w_full()
        .whitespace_nowrap()
        .child(div().w(px(14.)).flex_shrink_0().text_center().text_color(fg).child(sign))
        .child(div().text_color(fg).child(line.text.clone()));
    if let Some(tint) = tint {
        row = row.bg(tint);
    }
    row.into_any_element()
}
