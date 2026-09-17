//! The stop-all bar pinned above the sidebar footer while 2+ chats stream:
//! "N running — stop all" halts every in-flight reply at once (the composer's
//! per-chat stop already covers a single running chat).

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The "N running — stop all" bar, or `None` below 2 running chats — the
/// composer's per-chat stop already covers a single running chat. One ghost
/// button matching the selection bar's styling; the wrapper carries the
/// test/click target id.
pub(super) fn running_bar(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let n = ws.running_chats();
    if n < 2 {
        return None;
    }
    Some(
        div()
            .id("stop-all")
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                Button::new("stop-all-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Pause)
                    .label(format!("{n} running — stop all"))
                    .on_click(cx.listener(|this, _, _, cx| this.stop_all_replies(cx))),
            ),
    )
}
