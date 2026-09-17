//! The activity-center chrome: the titlebar bell with its unread badge and
//! the dropdown panel listing recent activity. Mounted by `Workspace::render`
//! as a full-window overlay so the panel floats above the sidebar and chat
//! pane; a transparent backdrop swallows outside clicks to dismiss it. The
//! panel itself — header, entry rows, footer actions — lives in
//! `super::activity_panel`.

use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

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
        .when(ws.activity_open, |d| d.child(super::activity_panel::panel(ws, cx)))
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

/// "now" / "5m" / "2h" / "3d" — coarse age for a row's trailing label;
/// shared with the bookmarks panel.
pub(crate) fn relative_time(at: SystemTime) -> String {
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
