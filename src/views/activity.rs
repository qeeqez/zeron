//! The activity-center chrome: the titlebar bell with its unread badge and
//! the dropdown panel listing recent activity. Mounted by `Workspace::render`
//! as a full-window overlay so the panel floats above the sidebar and chat
//! pane; a transparent backdrop swallows outside clicks to dismiss it.

use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::activity::{ActivityEntry, ActivityKind};
use crate::workspace::Workspace;

/// The bell sits in the unified top bar, right of the sidebar toggle — the
/// same spot whether the sidebar is open or collapsed. `left` clears the
/// traffic lights (72px) plus the toggle button.
const BELL_LEFT: f32 = 104.;

/// Full-window layer: dismiss backdrop, the bell, then the panel (paint
/// order = z order). The wrapper itself has no listeners, so with the panel
/// closed only the bell is hit-testable and everything below stays live.
pub(crate) fn activity_overlay(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("activity-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .when(ws.activity_open, |d| {
            d.child(
                div()
                    .id("activity-backdrop")
                    .test_support()
                    .absolute()
                    .inset_0()
                    // Swallow the whole click — mousedown alone would still
                    // let the synthesized click reach rows under the panel.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.activity_open {
                                this.toggle_activity_panel(cx);
                            }
                        }),
                    ),
            )
        })
        .child(bell(ws.activity.unread_count(), ws.activity_open, cx))
        .when(ws.activity_open, |d| d.child(panel(ws, cx)))
}

/// The bell button with its unread-count badge — a red dot carrying the
/// count (capped at 99+) so the icon stays legible at a glance.
fn bell(unread: usize, open: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .absolute()
        .left(px(BELL_LEFT))
        .top_0()
        .h(px(crate::window::TOP_BAR_H))
        .flex()
        .items_center()
        .child(
            div()
                .id("activity-bell")
                .test_support()
                .relative()
                .p_1()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                // The strip's mousedown arms a window move — stop it so the
                // press clicks the bell instead of dragging.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when(open, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
                .when(!open, |d| d.text_color(cx.theme().muted_foreground).hover(|d| d.bg(cx.theme().muted)))
                .child(IconName::Bell)
                .when(unread > 0, |d| {
                    d.child(
                        div()
                            .id("activity-badge")
                            .test_support()
                            .absolute()
                            .top(px(-4.))
                            .right(px(-6.))
                            .min_w(px(14.))
                            .h(px(14.))
                            .px(px(3.))
                            .rounded_full()
                            .bg(cx.theme().danger)
                            .text_color(cx.theme().danger_foreground)
                            .text_size(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if unread > 99 { "99+".to_string() } else { unread.to_string() }),
                    )
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_activity_panel(cx))),
        )
}

/// The dropdown: header (title + Clear + close) over the newest-first entry
/// list, capped in height so a long feed scrolls instead of covering the
/// window.
fn panel(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
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
        // Panel clicks must not fall through to the backdrop's dismiss.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
                        .id("activity-clear")
                        .test_support()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Clear")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_activity(cx))),
                )
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
                        div().p_3().text_sm().text_color(cx.theme().muted_foreground).child("No recent activity"),
                    )
                })
                .children(ws.activity.recent().map(|(ix, entry)| entry_row(ix, entry, cx))),
        )
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
                        .child(div().text_color(cx.theme().muted_foreground).text_size(px(10.)).child(relative_time(entry.at)))
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

/// "now" / "5m" / "2h" / "3d" — coarse age for the row's trailing label.
fn relative_time(at: SystemTime) -> String {
    let secs = at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}
