use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The empty/new-chat state — Codex-style: a quiet brand mark, a prompt, and
/// a 2×2 grid of suggestion chips that fill the composer and send.
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
        .gap_6()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_10()
                .rounded_xl()
                .bg(cx.theme().secondary)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Bot).size_5()),
        )
        .child(div().text_lg().font_weight(FontWeight::MEDIUM).child("What should we work on?"))
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_center()
                .gap_2()
                .max_w(px(520.))
                .children(suggestions.iter().map(|s| {
                    let ws = ws.clone();
                    let prompt = *s;
                    div()
                        .id(SharedString::from(format!("suggestion-{prompt}")))
                        .cursor_pointer()
                        .px_3()
                        .py_1p5()
                        .rounded_full()
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .hover(|style| style.bg(cx.theme().secondary).text_color(cx.theme().foreground))
                        .child(prompt)
                        .on_click(move |_, window, cx| {
                            ws.update(cx, |this, cx| {
                                this.composer.update(cx, |s, cx| s.set_value(prompt, window, cx));
                                this.send(window, cx);
                            });
                        })
                })),
        )
}
