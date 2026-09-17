//! The activity-center dropdown: a header (title + close) over the
//! newest-first entry list, an empty state, and a footer strip of bulk
//! actions — mark-all-read, clear-read, clear-all. The bell and its
//! full-window overlay live in `super::activity`, which mounts `panel`;
//! extracted here so both files stay under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::activity::{ActivityEntry, ActivityKind};
use crate::workspace::Workspace;

/// The dropdown, capped in height so a long feed scrolls instead of
/// covering the window. The list scrolls between the header and the
/// footer's bulk actions.
pub(crate) fn panel(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("activity-panel")
        .test_support()
        .absolute()
        .left(px(72.))
        .top(px(crate::window::TOP_BAR_H + 4.))
        .w(px(320.))
        .max_h(px(420.))
        .flex()
        .flex_col()
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .shadow_lg()
        // Panel clicks must not fall through to the backdrop's dismiss —
        // mouse-up too, since the backdrop closes on up, not down.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
                .child(IconName::Bell)
                .child("Activity")
                .child(div().flex_1())
                .child(
                    div()
                        .id("activity-close")
                        .test_support()
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::X)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_activity_panel(cx))),
                ),
        )
        .child(
            div()
                .id("activity-list")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_1()
                .flex()
                .flex_col()
                .when(ws.activity.entries.is_empty(), |d| {
                    d.child(
                        div()
                            .id("activity-empty")
                            .test_support()
                            .aria_label("No recent activity")
                            .p_3()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("No recent activity"),
                    )
                })
                .children(ws.activity.recent().map(|(ix, entry)| entry_row(ix, entry, cx))),
        )
        .when(!ws.activity.entries.is_empty(), |d| d.child(footer(ws, cx)))
}

/// The bulk-action strip under the list: "Mark all read" drops every dot
/// without opening a chat, "Clear read" sweeps acknowledged rows while
/// the unread survive, "Clear all" empties the feed. Each button renders
/// only while it would change something — an all-read feed shows just
/// "Clear all", an all-unread feed shows "Mark all read" + "Clear all".
fn footer(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let unread = ws.activity.unread_count();
    let has_read = ws.activity.entries.len() > unread;
    div()
        .id("activity-footer")
        .test_support()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .when(unread > 0, |d| {
            d.child(action("activity-mark-read", "Mark all read", Workspace::mark_all_activity_read, cx))
        })
        .child(div().flex_1())
        // With nothing unread, "Clear read" would only repeat "Clear all".
        .when(unread > 0 && has_read, |d| {
            d.child(action("activity-clear-read", "Clear read", Workspace::clear_read_activity, cx))
        })
        .child(action("activity-clear", "Clear all", Workspace::clear_activity, cx))
}

/// A muted text button — the same idiom the sibling panels use for their
/// header actions.
fn action(
    id: impl Into<ElementId>, label: &'static str, f: fn(&mut Workspace, &mut Context<Workspace>), cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .cursor_pointer()
        .text_color(cx.theme().muted_foreground)
        .hover(|d| d.text_color(cx.theme().foreground))
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            // Keep the click off anything behind the button.
            cx.stop_propagation();
            f(this, cx);
        }))
}

/// One feed row: kind icon, chat title + preview, a relative timestamp, an
/// unread dot, and a per-row dismiss ×. Clicking opens the chat (approval
/// entries also scroll to the card); the × removes the row without opening.
fn entry_row(ix: usize, entry: &ActivityEntry, cx: &mut Context<Workspace>) -> AnyElement {
    let (icon, color) = match entry.kind {
        ActivityKind::TurnFinished => (IconName::CircleCheck, cx.theme().success),
        ActivityKind::Approval => (IconName::ShieldAlert, cx.theme().warning),
        ActivityKind::Error => (IconName::CircleX, cx.theme().danger),
        ActivityKind::Note => (IconName::TriangleAlert, cx.theme().warning),
    };
    div()
        .id(("activity-entry", ix))
        .test_support()
        .flex()
        .items_start()
        .gap_2()
        .px_2()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.bg(cx.theme().muted))
        .child(div().pt(px(1.)).text_color(color).child(icon))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(entry.unread, |d| {
                            d.child(
                                div()
                                    .id(("activity-unread", ix))
                                    .test_support()
                                    .flex_shrink_0()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(cx.theme().accent),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .font_semibold()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(entry.chat_title.clone()),
                        )
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .text_size(px(10.))
                                .child(super::activity::relative_time(entry.at)),
                        )
                        .child(
                            div()
                                .id(("activity-dismiss", ix))
                                .test_support()
                                .flex_shrink_0()
                                .cursor_pointer()
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground)
                                .hover(|d| d.text_color(cx.theme().foreground))
                                .child(IconName::X)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    // Keep the click off the row — dismissing must not open the chat.
                                    cx.stop_propagation();
                                    this.dismiss_activity_entry(ix, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(entry.body.clone()),
                ),
        )
        .on_click(cx.listener(move |this, _, window, cx| this.open_activity_entry(ix, window, cx)))
        .into_any_element()
}
