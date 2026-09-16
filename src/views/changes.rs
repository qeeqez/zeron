//! Changes-panel rendering: the header, file rows with per-file stage
//! toggles, expanded inline diffs, the review banner, and the git action
//! block (branch, commit box, push/PR). State and git ops live in
//! `crate::changes`; diff bodies in `crate::views::diff`.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes_diff::DiffMode;
use crate::git::{ChangeStatus, FileChange};
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_changes_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut next_line = 0usize;
        let rows: Vec<AnyElement> = self
            .changes
            .iter()
            .enumerate()
            .map(|(ix, c)| change_entry(ix, c, &mut next_line, self, cx))
            .collect();
        let summary = crate::changes_diff::diff_summary(&self.changes);
        div()
            .id("changes-panel")
            .test_support()
            .w(px(360.))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .text_sm()
                            .font_bold()
                            .child(IconName::FileDiff)
                            .child("Changes")
                            .child(div().flex_1())
                            .child(diff_mode_toggle(self.diff_mode, cx))
                            .child(
                                div()
                                    .id("refresh-changes")
                                    .test_support()
                                    .cursor_pointer()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(IconName::RefreshCcw)
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh_changes(cx))),
                            )
                            .child(
                                div()
                                    .id("close-changes")
                                    .test_support()
                                    .cursor_pointer()
                                    .child(IconName::X)
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_changes_panel(cx))),
                            ),
                    )
                    // `K files changed · +N −M` — the same rollup real Codex
                    // shows under the panel title; hidden on a clean tree.
                    .when_some(summary, |d, s| {
                        d.child(
                            div()
                                .id("changes-summary")
                                .test_support()
                                .aria_label(s.text())
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .pb_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} files changed ·", s.files))
                                .child(div().text_color(cx.theme().success).child(format!("+{}", s.added)))
                                .child(div().text_color(cx.theme().danger).child(format!("−{}", s.deleted))),
                        )
                    }),
            )
            .when(!self.review.comments.is_empty(), |d| d.child(crate::views::diff::review_banner(self, cx)))
            .child(
                div()
                    .id("changes-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(rows.is_empty(), |d| {
                        d.child(div().text_sm().text_color(cx.theme().muted_foreground).child("No changes — working tree is clean"))
                    })
                    .children(rows),
            )
            .when_some(self.git.branch.clone(), |d, branch| d.child(crate::views::changes_git::git_block(self, &branch, cx)))
    }
}

/// A file row plus, when expanded, its inline diff. `next_line` hands out
/// unique `("diff-line", n)` ids across every expanded file in the panel.
fn change_entry(ix: usize, change: &FileChange, next_line: &mut usize, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let mut entry = div().flex().flex_col().child(change_row(ix, change, ws, cx));
    if change.diff.is_some() {
        entry = entry.child(crate::views::diff::render_diff(ix, change, next_line, ws, cx));
    }
    entry.into_any_element()
}

fn change_row(ix: usize, change: &FileChange, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let (icon, color) = match change.status {
        ChangeStatus::Added => (IconName::FilePlus, cx.theme().success),
        ChangeStatus::Modified => (IconName::FilePen, cx.theme().warning),
        ChangeStatus::Deleted => (IconName::FileX, cx.theme().danger),
        ChangeStatus::Renamed => (IconName::FileSymlink, cx.theme().info),
        ChangeStatus::Conflicted => (IconName::CircleAlert, cx.theme().danger),
    };
    let expanded = change.diff.is_some();
    div()
        .id(("change-row", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .hover(|d| d.bg(cx.theme().muted))
        // The stage toggle only exists in a repo — `ws.git.branch` is the
        // same probe that gates the commit/push block below the list.
        .when(ws.git.branch.is_some(), |d| {
            d.child(
                div()
                    .id(("stage-toggle", ix))
                    .test_support()
                    .flex_shrink_0()
                    .text_color(if change.staged { cx.theme().accent } else { cx.theme().muted_foreground })
                    .child(if change.staged { IconName::SquareCheck } else { IconName::Square })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // Keep the click off the row — it must not expand the diff.
                        cx.stop_propagation();
                        this.toggle_change_stage(ix, cx);
                    })),
            )
        })
        .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(if expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        }))
        .child(div().flex_shrink_0().text_color(color).child(icon))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis_middle()
                .child(change.path.clone()),
        )
        .when(change.added > 0, |d| {
            d.child(div().flex_shrink_0().text_xs().text_color(cx.theme().success).child(format!("+{}", change.added)))
        })
        .when(change.deleted > 0, |d| {
            d.child(div().flex_shrink_0().text_xs().text_color(cx.theme().danger).child(format!("-{}", change.deleted)))
        })
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_change_diff(ix, cx)))
        .context_menu({
            let ws = cx.entity();
            let path = change.path.clone();
            move |menu, window, cx| crate::open_in::file_menu(&ws, &path, menu, window, cx)
        })
        .into_any_element()
}

/// The unified|split segmented control in the panel header. The active
/// segment carries the accent fill; clicking either segment persists the
/// mode via `set_diff_mode`.
fn diff_mode_toggle(mode: DiffMode, cx: &mut Context<Workspace>) -> impl IntoElement {
    // Copy theme fields up front — `cx.theme()` borrows `*cx` and the
    // segment builders below need `&mut cx` for their click listeners.
    let (border, muted_fg, accent, accent_fg) = {
        let theme = cx.theme();
        (theme.border, theme.muted_foreground, theme.accent, theme.accent_foreground)
    };
    let segment = |id: &'static str, label: &'static str, segment_mode: DiffMode| {
        let mut seg = div().id(id).test_support().cursor_pointer().px_2().py_0p5().text_xs().child(label);
        seg = if segment_mode == mode { seg.bg(accent).text_color(accent_fg) } else { seg.text_color(muted_fg) };
        seg.on_click(cx.listener(move |this, _, _, cx| this.set_diff_mode(segment_mode, cx)))
    };
    div()
        .flex()
        .items_center()
        .rounded_md()
        .overflow_hidden()
        .border_1()
        .border_color(border)
        .child(segment("diff-mode-unified", "Unified", DiffMode::Unified))
        .child(div().w(px(1.)).h_full().bg(border))
        .child(segment("diff-mode-split", "Split", DiffMode::Split))
}
