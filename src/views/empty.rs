use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

pub fn render_empty_state(ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let suggestions = [
        "Explain this codebase",
        "Fix the failing tests",
        "Refactor the parser module",
        "Write docs for the public API",
    ];
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_4()
        .child(div().text_lg().text_color(cx.theme().muted_foreground).child("What should we work on?"))
        .child(
            div()
                .id("empty-new-chat")
                .cursor_pointer()
                .px_4()
                .py_2()
                .rounded_lg()
                .bg(cx.theme().accent)
                .text_color(cx.theme().accent_foreground)
                .text_sm()
                .child("New chat")
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.new_chat(cx));
                }),
        )
        .child(div().flex().flex_col().gap_2().items_center().children(suggestions.iter().map(|s| {
            div()
                .px_4()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(*s)
        })))
}
