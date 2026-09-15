use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{ChangeStatus, FileChange};
use crate::workspace::Workspace;

/// Token source for in-flight row-diff loads — each expand stamps the row
/// with a fresh id so a stale result can't attach after collapse+re-expand.
static NEXT_DIFF_LOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What a background diff load was issued under: `(change-list generation,
/// row load token)`. Both must still match when the result lands — a refresh
/// bumps the generation, collapse/re-expand changes the token — or the diff
/// is stale and gets discarded.
type DiffStamp = (u64, u64);

impl Workspace {
    /// Expand/collapse a row's inline diff. Expanding stamps the row with a
    /// load token and fetches the working-tree diff on the background
    /// executor; collapsing drops the cached diff and clears the token so a
    /// still-running load is discarded when it lands.
    pub fn toggle_change_diff(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.changes.get(ix).is_some_and(|c| c.diff.is_some() || c.diff_load != 0) {
            let row = &mut self.changes[ix];
            row.diff = None;
            row.diff_load = 0;
            cx.notify();
            return;
        }
        let stamp: DiffStamp = (self.changes_generation, NEXT_DIFF_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let row = &mut self.changes[ix];
        row.diff_load = stamp.1;
        let change = row.clone();
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let diff = cx
                .background_executor()
                .spawn(async move { crate::changes_diff::diff_for_file(&dir, &change) })
                .await;
            let _ = this.update(cx, |this, cx| this.land_change_diff(stamp, diff, cx));
        })
        .detach();
    }

    /// Store a loaded diff on the row stamped with the stamp's token —
    /// skipped when the list generation moved on (a refresh landed or is in
    /// flight) or no row still waits on that token (collapsed or re-expanded
    /// under the load). The token is unique per load, so it identifies the row.
    pub(crate) fn land_change_diff(&mut self, stamp: DiffStamp, diff: Option<crate::changes_diff::FileDiff>, cx: &mut Context<Self>) {
        if stamp.0 != self.changes_generation {
            return;
        }
        let Some(row) = self.changes.iter_mut().find(|r| r.diff_load == stamp.1) else { return };
        row.diff_load = 0;
        row.diff = diff;
        cx.notify();
    }

    /// Re-run git collection for the Changes panel. Collection shells out to
    /// several git processes and reads untracked files, so it runs on the
    /// background executor and publishes the result back when done.
    pub fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        self.changes_generation += 1;
        let generation = self.changes_generation;
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let changes = cx.background_executor().spawn(async move { crate::git::collect(&root) }).await;
            let _ = this.update(cx, |this, cx| this.land_changes(generation, changes, cx));
        })
        .detach();
    }

    /// Publish a collected change list — skipped when a newer refresh was
    /// requested while this one ran, so an older result can't revert the
    /// panel to a stale snapshot.
    pub(crate) fn land_changes(&mut self, generation: u64, changes: Vec<FileChange>, cx: &mut Context<Self>) {
        if generation != self.changes_generation {
            return;
        }
        self.changes = changes;
        cx.notify();
    }

    pub fn render_changes_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut next_line = 0usize;
        let rows: Vec<AnyElement> = self
            .changes
            .iter()
            .enumerate()
            .map(|(ix, c)| change_entry(ix, c, &mut next_line, self, cx))
            .collect();
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
fn change_entry(ix: usize, change: &FileChange, next_line: &mut usize, ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let mut entry = div().flex().flex_col().child(change_row(ix, change, cx));
    if let Some(diff) = &change.diff {
        entry = entry.child(crate::views::diff::render_diff(ix, diff, next_line, ws, cx));
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
