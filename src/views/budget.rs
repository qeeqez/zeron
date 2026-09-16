//! The budget-alert banner under the transcript: a warning row when the
//! chat's accumulated spend crosses its cap ("This chat has spent ~$1.25
//! (cap $1)"). Dismiss hides it until the cap changes — see
//! `crate::chat_ops::budget`.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The banner row: spend vs cap plus a Dismiss affordance.
pub fn budget_banner(spent: f64, cap: f64, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let label = format!("This chat has spent ~{} (cap {})", crate::pricing::fmt_cost(spent), crate::pricing::fmt_cost(cap));
    let ws = ws.clone();
    div()
        .id("budget-banner")
        .test_support()
        .aria_label(label.clone())
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_1()
        .text_xs()
        .text_color(cx.theme().warning)
        .child(IconName::CircleDollarSign)
        .child(label)
        .child(
            div()
                .id("dismiss-budget")
                .test_support()
                .cursor_pointer()
                .underline()
                .child("Dismiss")
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.dismiss_budget_alert(cx));
                }),
        )
}
