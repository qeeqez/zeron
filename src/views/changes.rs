use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{ChangeStatus, FileChange};
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_changes_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self.changes.iter().enumerate().map(|(ix, c)| change_row(ix, c, cx)).collect();
        let added: u32 = self.changes.iter().map(|c| c.added).sum();
        let deleted: u32 = self.changes.iter().map(|c| c.deleted).sum();

        div()
            .id("changes-panel")
            .test_support()
            .w(px(280.))
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

fn change_row(ix: usize, change: &FileChange, cx: &mut Context<Workspace>) -> AnyElement {
    let (icon, color) = match change.status {
        ChangeStatus::Added => (IconName::FilePlus, cx.theme().success),
        ChangeStatus::Modified => (IconName::FilePen, cx.theme().warning),
        ChangeStatus::Deleted => (IconName::FileX, cx.theme().danger),
        ChangeStatus::Renamed => (IconName::FileSymlink, cx.theme().info),
        ChangeStatus::Conflicted => (IconName::CircleAlert, cx.theme().danger),
    };
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
        .into_any_element()
}
