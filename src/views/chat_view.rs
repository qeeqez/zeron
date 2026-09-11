use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageHeader};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::cards::{render_diff, render_tool_call};
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = &self.chats[self.active];
        let empty = chat.messages.is_empty();
        let messages: Rc<Vec<ChatMessage>> = Rc::new(chat.messages.clone());
        let running = chat.running;
        let title = chat.title.clone();
        let ws = cx.entity();
        let ws_toggle = cx.entity();

        let running_agents = self.running_agents();
        let panel_open = self.agents_panel_open;

        let list = MessageScroller::new("chat-messages", self.scroller.clone(), move |ix, _window, cx| {
            messages
                .get(ix)
                .map(|msg| render_message(ix, msg, &ws, cx))
                .unwrap_or_else(|| div().into_any_element())
        });
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child(title)
            .child(div().flex_1())
            .child(
                div()
                    .id("agents-toggle")
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .when(panel_open, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
                    .when(!panel_open, |d| d.text_color(cx.theme().muted_foreground))
                    .child(IconName::Bot)
                    .when(running_agents > 0, |d| d.child(format!("{running_agents}")))
                    .on_click(move |_, _, cx| {
                        ws_toggle.update(cx, |this, cx| this.toggle_agents_panel(cx));
                    }),
            );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(if empty { render_empty_state(cx).into_any_element() } else { list.into_any_element() }),
            )
            .when(running, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::LoaderCircle)
                        .child("Working…"),
                )
            })
            .child(self.render_composer(cx))
    }
}

fn render_message(ix: usize, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    match &msg.kind {
        MessageKind::Text(text) => render_text(ix, msg.role, text, cx),
        MessageKind::Tool(tool) => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
    }
}

fn render_empty_state(cx: &mut App) -> impl IntoElement {
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

fn render_text(ix: usize, role: Role, text: &SharedString, cx: &mut App) -> AnyElement {
    let alignment = match role {
        Role::User => MessageAlignment::End,
        Role::Assistant => MessageAlignment::Start,
    };
    let body = div()
        .px_4()
        .py_2()
        .rounded_lg()
        .text_sm()
        .when(role == Role::User, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
        .when(role == Role::Assistant, |d| d.bg(cx.theme().secondary).text_color(cx.theme().foreground))
        .child(if role == Role::Assistant {
            TextView::markdown(("md", ix), text.clone()).into_any_element()
        } else {
            div().child(text.clone()).into_any_element()
        });

    let mut message = Message::new().alignment(alignment).content(MessageContent::new().child(body));
    if role == Role::Assistant {
        message = message.header(
            MessageHeader::new().child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(IconName::Bot)
                    .child("Rixl"),
            ),
        );
    }
    message.into_any_element()
}
