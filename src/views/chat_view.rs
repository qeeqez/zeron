use std::rc::Rc;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::cards::{MsgCtx, message_footer, render_diff, render_tool_call};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub fn render_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = &self.chats[self.active];
        let empty = chat.messages.is_empty();
        let messages: Rc<Vec<ChatMessage>> = Rc::new(chat.messages.clone());
        let running = chat.running;
        let failed = chat.failed_flag;
        let title = chat.title.clone();
        let pinned = chat.pinned;
        let ws = cx.entity();
        let ws_empty = cx.entity();
        let ws_menu = cx.entity();
        let ws_toggle = cx.entity();

        let running_agents = self.running_agents();
        let panel_open = self.agents_panel_open;

        let msg_count = messages.len();
        let list = MessageScroller::new("chat-messages", self.scroller.clone(), move |ix, _window, cx| {
            messages
                .get(ix)
                .map(|msg| render_message(MsgCtx { ix, is_last: ix == msg_count - 1 }, msg, &ws, cx))
                .unwrap_or_else(|| div().into_any_element())
        })
        .jump_button(true)
        .with_jump_button_label("Jump to latest");

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
            )
            .child(Button::new("chat-menu").ghost().icon(IconName::Ellipsis).dropdown_menu(move |menu, _window, _cx| {
                let ws_rename = ws_menu.clone();
                let ws_export = ws_menu.clone();
                let ws_copy = ws_menu.clone();
                let ws_pin = ws_menu.clone();
                let pin_label = if pinned { "Unpin" } else { "Pin" };
                menu.item(PopupMenuItem::new(pin_label).icon(IconName::Star).on_click(move |_, _, cx| {
                    ws_pin.update(cx, |this, cx| this.toggle_pin(this.active, cx));
                }))
                .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, window, cx| {
                    ws_rename.update(cx, |this, cx| this.rename_active(window, cx));
                }))
                .item(PopupMenuItem::new("Export").icon(IconName::Share).on_click(move |_, _, cx| {
                    ws_export.update(cx, |this, cx| this.export_active(cx));
                }))
                .item(PopupMenuItem::new("Copy transcript").icon(IconName::Copy).on_click(move |_, _, cx| {
                    ws_copy.update(cx, |this, cx| this.copy_transcript(cx));
                }))
            }));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(header)
            .child(div().flex_1().min_h_0().child(if empty {
                render_empty_state(ws_empty.clone(), cx).into_any_element()
            } else {
                list.into_any_element()
            }))
            .when(running, |d| {
                let elapsed = chat.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
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
                        .child(format!("Working… {elapsed}s")),
                )
            })
            .when(failed && !running, |d| {
                let ws_retry = ws_empty.clone();
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(IconName::TriangleAlert)
                        .child("Reply failed")
                        .child(div().id("retry-failed").cursor_pointer().underline().child("Retry").on_click(move |_, _, cx| {
                            ws_retry.update(cx, |this, cx| this.retry_last(cx));
                        })),
                )
            })
            .child(self.render_composer(cx))
    }
}

fn render_message(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, .. } = mc;
    match &msg.kind {
        MessageKind::Text(_) => render_text(mc, msg, ws, cx),
        MessageKind::Tool(tool) => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
    }
}
fn render_empty_state(ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
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

fn render_text(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, .. } = mc;
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let role = msg.role;
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
    message = message.footer(MessageFooter::new().child(message_footer(mc, msg, ws, cx)));
    message.into_any_element()
}
