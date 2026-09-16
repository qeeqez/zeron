//! The rate-limit banner under the transcript: a warning row when the
//! backend throttled a turn ("Rate limited — resets 3:04 PM") or a quota
//! window is nearly full ("Approaching rate limit — 92% used"). It clears
//! when the next turn succeeds; a Retry affordance rides along while the
//! turn is idle, replacing the generic "Reply failed" row.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::rate_limit::RateLimit;
use crate::workspace::Workspace;

/// The banner row, or `None` when nothing is limited or nearly full.
pub fn rate_limit_banner(rl: &RateLimit, running: bool, ws: &Entity<Workspace>, cx: &App) -> Option<impl IntoElement> {
    let label = rl.banner(std::time::SystemTime::now())?;
    let ws = ws.clone();
    Some(
        div()
            .id("rate-limit-banner")
            .test_support()
            .aria_label(label.clone())
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_1()
            .text_xs()
            .text_color(cx.theme().warning)
            .child(IconName::TriangleAlert)
            .child(label)
            .when(rl.limited && !running, |d| {
                d.child(div().id("retry-rate-limit").test_support().cursor_pointer().underline().child("Retry").on_click(
                    move |_, _, cx| {
                        ws.update(cx, |this, cx| this.retry_last(cx));
                    },
                ))
            }),
    )
}
