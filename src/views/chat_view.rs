use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageHeader};
use gpui_kit::component::message_scroller::MessageScroller;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let messages: Rc<Vec<ChatMessage>> = Rc::new(self.chats[self.active].messages.clone());
        let running = self.chats[self.active].running;

        let list = MessageScroller::new("chat-messages", self.scroller.clone(), move |ix, window, cx| {
            messages
                .get(ix)
                .map(|msg| render_message(msg, window, cx))
                .unwrap_or_else(|| div().into_any_element())
        });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(div().flex_1().min_h_0().child(list))
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

fn render_message(msg: &ChatMessage, _window: &mut Window, cx: &mut App) -> AnyElement {
    match &msg.kind {
        MessageKind::Text(text) => render_text(msg.role, text, cx),
        MessageKind::Tool(tool) => render_tool_call(tool, cx).into_any_element(),
    }
}

fn render_text(role: Role, text: &SharedString, cx: &mut App) -> AnyElement {
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
        .child(text.clone());

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

fn render_tool_call(tool: &ToolCall, cx: &mut App) -> impl IntoElement {
    let (icon, status_color) = match tool.status {
        ToolStatus::Running => (IconName::LoaderCircle, cx.theme().info),
        ToolStatus::Done => (IconName::CircleCheck, cx.theme().success),
        ToolStatus::Failed => (IconName::CircleX, cx.theme().danger),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .text_sm()
        .child(div().text_color(status_color).child(icon))
        .child(IconName::SquareTerminal)
        .child(tool.name.clone())
        .child(div().text_color(cx.theme().muted_foreground).child(tool.detail.clone()))
}
