//! The Bookmarks panel — a right-side list of every bookmarked message in
//! the project's loaded chats, grouped per chat in sidebar order. Rows
//! jump to their message (`Workspace::open_bookmark`); the × unstars in
//! place and the header's "Clear all" empties the list. State and the
//! cross-chat scan live in `crate::chat_msg::bookmarks_panel`.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chat_msg::bookmarks_panel::BookmarkRow;
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_bookmarks_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.bookmark_groups();
        let empty = groups.is_empty();
        let mut body: Vec<AnyElement> = Vec::new();
        for group in groups {
            body.push(
                div()
                    .id(format!("bm-group-{}", group.chat_id))
                    .test_support()
                    .aria_label(group.title.clone())
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(group.title.clone())
                    .into_any_element(),
            );
            body.extend(group.rows.iter().map(|row| bookmark_row(row, cx).into_any_element()));
        }

        div()
            .id("bookmarks-panel")
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
                    .child(IconName::Bookmark)
                    .child("Bookmarks")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("clear-bookmarks")
                            .test_support()
                            .cursor_pointer()
                            .text_xs()
                            .font_normal()
                            .text_color(cx.theme().muted_foreground)
                            .hover(|d| d.text_color(cx.theme().foreground))
                            .child("Clear all")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_all_bookmarks(cx))),
                    )
                    .child(
                        div()
                            .id("close-bookmarks")
                            .test_support()
                            .cursor_pointer()
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_bookmarks_panel(cx))),
                    ),
            )
            .child(
                div()
                    .id("bookmarks-list")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py_1()
                    .flex()
                    .flex_col()
                    .when(empty, |d| {
                        d.child(
                            div()
                                .id("bookmarks-empty")
                                .test_support()
                                .aria_label("No bookmarks")
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No bookmarks — star a message to keep it here"),
                        )
                    })
                    .children(body),
            )
    }
}

/// One bookmarked message: its snippet on the left, the relative time and a
/// hover × on the right. The row click jumps to the message; the × unstars
/// without jumping (propagation stopped, same as the activity feed's
/// dismiss button).
fn bookmark_row(row: &BookmarkRow, cx: &mut Context<Workspace>) -> impl IntoElement {
    let chat_id = row.chat_id;
    let msg_ix = row.msg_ix;
    div()
        .id(format!("bm-row-{chat_id}-{msg_ix}"))
        .test_support()
        .aria_label(row.snippet.clone())
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.bg(cx.theme().muted))
        .child(div().flex_1().min_w_0().text_sm().whitespace_nowrap().text_ellipsis().child(row.snippet.clone()))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(super::activity::relative_time(row.at)),
        )
        .child(
            div()
                .id(format!("bm-unmark-{chat_id}-{msg_ix}"))
                .test_support()
                .flex_shrink_0()
                .cursor_pointer()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .hover(|d| d.text_color(cx.theme().foreground))
                .child(IconName::X)
                .on_click(cx.listener(move |this, _, _, cx| {
                    // Keep the click off the row — unstarring must not jump.
                    cx.stop_propagation();
                    this.unbookmark(chat_id, msg_ix, cx);
                })),
        )
        .on_click(cx.listener(move |this, _, window, cx| this.open_bookmark(chat_id, msg_ix, window, cx)))
}

/// The sidebar's Bookmarks row — opens the panel; the suffix is the total
/// star count across loaded chats (hidden while zero). Extracted so
/// `sidebar.rs` stays under the SLOC cap.
pub(crate) fn bookmarks_nav_row(ws: &Workspace, cx: &mut Context<Workspace>) -> super::nav_row::NavRow {
    let count = ws.bookmark_count();
    super::nav_row::NavRow::new("sidebar-bookmarks", "Bookmarks")
        .icon(IconName::Bookmark)
        .active(ws.bookmarks_panel.open)
        .suffix(move |_, cx| {
            div()
                .id("sidebar-bookmarks-count")
                .test_support()
                .text_xs()
                .when(count > 0, |d| d.aria_label(count.to_string()).text_color(cx.theme().muted_foreground).child(count.to_string()))
        })
        .on_click(cx.listener(|this, _, _, cx| this.toggle_bookmarks_panel(cx)))
}
