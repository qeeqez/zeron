use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{ChangeStatus, FileChange};
use crate::workspace::Workspace;

impl Workspace {
    /// Expand/collapse a row's inline diff. Expanding loads the working-tree
    /// diff on the background executor and caches it on the row; collapsing
    /// drops it.
    pub fn toggle_change_diff(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.changes.get(ix).is_some_and(|c| c.diff.is_some()) {
            self.changes[ix].diff = None;
            cx.notify();
            return;
        }
        let Some(change) = self.changes.get(ix).cloned() else { return };
        let dir = self.project.root().to_path_buf();
        let path = change.path.clone();
        cx.spawn(async move |this, cx| {
            let diff = cx
                .background_executor()
                .spawn(async move { crate::changes_diff::diff_for_file(&dir, &change) })
                .await;
            let _ = this.update(cx, |this, cx| this.land_change_diff(&path, diff, cx));
        })
        .detach();
    }

    /// Store a loaded diff on the row for `path` — skipped when the list
    /// refreshed under the load and that file is no longer listed.
    fn land_change_diff(&mut self, path: &str, diff: Option<crate::changes_diff::FileDiff>, cx: &mut Context<Self>) {
        let Some(row) = self.changes.iter_mut().find(|r| r.path == path) else { return };
        row.diff = diff;
        cx.notify();
    }

    /// Re-run git collection for the Changes panel. Collection shells out to
    /// several git processes and reads untracked files, so it runs on the
    /// background executor and publishes the result back when done.
    pub fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let changes = cx.background_executor().spawn(async move { crate::git::collect(&root) }).await;
            let _ = this.update(cx, |this, cx| {
                this.changes = changes;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn render_changes_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut next_line = 0usize;
        let rows: Vec<AnyElement> = self.changes.iter().enumerate().map(|(ix, c)| change_entry(ix, c, &mut next_line, cx)).collect();
        let added: u32 = self.changes.iter().map(|c| c.added).sum();
        let deleted: u32 = self.changes.iter().map(|c| c.deleted).sum();
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
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .font_bold()
                    .child(IconName::FileDiff)
                    .child("Changes")
                    .child(div().flex_1())
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} files", self.changes.len()))
                    .child(div().flex_1())
                    .child(div().text_color(cx.theme().success).child(format!("+{added}")))
                    .child(div().text_color(cx.theme().danger).child(format!("-{deleted}"))),
            )
    }
}

/// A file row plus, when expanded, its inline diff. `next_line` hands out
/// unique `("diff-line", n)` ids across every expanded file in the panel.
fn change_entry(ix: usize, change: &FileChange, next_line: &mut usize, cx: &mut Context<Workspace>) -> AnyElement {
    let mut entry = div().flex().flex_col().child(change_row(ix, change, cx));
    if let Some(diff) = &change.diff {
        entry = entry.child(crate::views::diff::render_diff(ix, diff, next_line, cx));
    }
    entry.into_any_element()
}

fn change_row(ix: usize, change: &FileChange, cx: &mut Context<Workspace>) -> AnyElement {
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
        .into_any_element()
}
