//! The pinned-message banner under the chat titlebar: pin icon, the
//! message's snippet, and a × that unpins. Clicking the row scrolls the
//! transcript to the message (see `Workspace::toggle_message_pin`).

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The banner row: pin icon + snippet; click jumps, × unpins.
pub fn pinned_banner(ix: usize, snippet: String, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let ws_jump = ws.clone();
    let ws_unpin = ws.clone();
    div()
        .id("pinned-banner")
        .test_support()
        .aria_label(format!("Pinned: {snippet}"))
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_1()
        .border_b_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .cursor_pointer()
        .child(IconName::Pin)
        .child(div().flex_1().min_w_0().whitespace_nowrap().text_ellipsis().child(snippet))
        .on_click(move |_, _, cx| {
            ws_jump.update(cx, |this, cx| this.scroll_to_message(ix, cx));
        })
        .child(
            div()
                .id("unpin")
                .test_support()
                .flex_shrink_0()
                .cursor_pointer()
                .hover(|d| d.text_color(cx.theme().foreground))
                .child(IconName::X)
                .on_click(move |_, _, cx| {
                    // Keep the click off the row — unpinning must not jump.
                    cx.stop_propagation();
                    ws_unpin.update(cx, |this, cx| this.unpin_message(cx));
                }),
        )
}
