//! The floating preview that follows the cursor while a chat row drags — a
//! small pill with the chat's title, styled like the row it came from.
//! Split from `sidebar_row.rs` for the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

pub(super) struct ChatDragGhost {
    pub title: SharedString,
}

impl Render for ChatDragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("chat-drag-ghost")
            .cursor_grabbing()
            .gap_2()
            .py_1()
            .px_3()
            .max_w_64()
            .overflow_hidden()
            .whitespace_nowrap()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tokens.sidebar)
            .text_sm()
            .text_color(cx.theme().sidebar_foreground)
            .opacity(0.85)
            .child(IconName::FileText)
            .child(self.title.clone())
    }
}
