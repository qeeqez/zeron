use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::cards::{MsgCtx, message_footer, render_diff, render_tool_call};
use crate::workspace::Workspace;

pub fn render_message(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, .. } = mc;
    match &msg.kind {
        MessageKind::Text(_) => render_text(mc, msg, ws, cx),
        MessageKind::Tool(tool) => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
    }
}

fn render_text(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, .. } = mc;
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let role = msg.role;
    let word_wrap = ws.read(cx).word_wrap;
    let font_size = ws.read(cx).font_size;
    let alignment = match role {
        Role::User => MessageAlignment::End,
        Role::Assistant => MessageAlignment::Start,
    };
    let body = div()
        .px_4()
        .py_2()
        .rounded_lg()
        .text_size(px(f32::from(font_size)))
        .when(role == Role::User, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
        .when(role == Role::Assistant, |d| d.bg(cx.theme().secondary).text_color(cx.theme().foreground))
        .child(if role == Role::Assistant {
            TextView::markdown(("md", ix), text.clone())
                .selectable(true)
                .code_block_actions(|block, _window, _cx| {
                    let code = block.code().to_string();
                    div()
                        .id("copy-code")
                        .cursor_pointer()
                        .text_color(hsla(0.0, 0.0, 0.55, 1.0))
                        .child(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                        })
                })
                .into_any_element()
        } else {
            div()
                .whitespace_nowrap()
                .when(word_wrap, |d| d.whitespace_normal())
                .child(text.clone())
                .into_any_element()
        });

    let mut message = Message::new().alignment(alignment).content(MessageContent::new().child(body));
    // Group for hover-revealed footer actions.
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
    let ws_menu = ws.clone();
    div()
        .group(SharedString::from(format!("msg-{ix}")))
        .child(message)
        .context_menu(move |menu, _window, _cx| {
            let ws_copy = ws_menu.clone();
            let ws_retry = ws_menu.clone();
            menu.item(
                gpui_kit::component::menu::PopupMenuItem::new("Copy")
                    .icon(IconName::Copy)
                    .on_click(move |_, _, cx| {
                        ws_copy.update(cx, |this, cx| this.copy_message(ix, cx));
                    }),
            )
            .item(
                gpui_kit::component::menu::PopupMenuItem::new("Retry")
                    .icon(IconName::RotateCcw)
                    .on_click(move |_, _, cx| {
                        ws_retry.update(cx, |this, cx| this.retry_last(cx));
                    }),
            )
        })
        .into_any_element()
}
