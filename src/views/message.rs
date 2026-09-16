use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{MessageKind, Role};

use crate::views::approval::render_approval;
use crate::views::cards::{MsgCtx, render_diff, render_plan, render_tool_call};
use crate::workspace::Workspace;

pub fn render_message(mc: MsgCtx, focused: bool, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    let el = match &msg.kind {
        MessageKind::Text(_) => render_text(mc, ws, window, cx),
        MessageKind::Tool(tool) => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
        MessageKind::Plan(plan) => render_plan(ix, plan, cx).into_any_element(),
        MessageKind::Approval(card) => render_approval(ix, card, ws.clone(), cx).into_any_element(),
    };
    crate::msg_nav::wrap_nav_focus(el, focused, cx)
}

fn render_text(mc: MsgCtx, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let role = msg.role;
    let (word_wrap, font_size, checkpointed, edit_input) = {
        let ws = ws.read(cx);
        let chat = &ws.chats[ws.active];
        let input = ws
            .editing
            .as_ref()
            .filter(|e| e.chat_id == chat.id && e.ix == ix && e.at == msg.at)
            .map(|e| e.input.clone());
        (ws.word_wrap, ws.font_size, input.is_none() && !chat.running && crate::checkpoints::for_message(chat, ix).is_some(), input)
    };
    let alignment = match role {
        Role::User => MessageAlignment::End,
        Role::Assistant => MessageAlignment::Start,
    };
    // Assistant bodies carry a keyed MarkdownState — the footer reads it for
    // the view-raw toggle, so it is created here and shared.
    let md_state = if role == Role::Assistant {
        Some(super::markdown::markdown_state(ix, text, window, cx))
    } else {
        None
    };
    // While the message is being edited the bubble becomes an inline
    // editor — Enter resends, Esc cancels (see `views::message_edit`).
    let body = if let Some(input) = edit_input {
        super::message_edit::message_editor(ix, &input, ws, cx)
    } else {
        // Image attachments render as thumbnails inside the bubble — a click
        // opens the lightbox (see `crate::image_view`).
        let thumbs: Vec<AnyElement> = msg
            .attachments
            .iter()
            .enumerate()
            .filter(|(_, a)| crate::attachment::is_image_path(a))
            .map(|(j, a)| {
                let ws_thumb = ws.clone();
                let path = a.to_string();
                div()
                    .id(SharedString::from(format!("msg-thumb-{ix}-{j}")))
                    .test_support()
                    .cursor_pointer()
                    .child(
                        img(std::path::PathBuf::from(&path))
                            .size(px(96.))
                            .rounded_md()
                            .object_fit(ObjectFit::Cover)
                            .with_fallback(|| IconName::Image.into_any_element()),
                    )
                    .on_click(move |_, _, cx| {
                        ws_thumb.update(cx, |this, cx| this.open_image_view(path.clone(), cx));
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .id(("md-body", ix))
            .test_support()
            .px_4()
            .py_2()
            .text_size(px(f32::from(font_size)))
            // Codex: user text sits in a tinted bubble; assistant replies are
            // flat Markdown on the chat surface — no bubble.
            .when(role == Role::User, |d| {
                d.rounded_lg().bg(cx.theme().accent).text_color(cx.theme().accent_foreground)
            })
            .when(!thumbs.is_empty(), |d| {
                d.child(div().flex().flex_wrap().gap_2().pb_1().children(thumbs))
            })
            .child(if let Some(state) = &md_state {
                super::markdown::assistant_markdown(ix, text, state, cx)
            } else {
                div()
                    .whitespace_nowrap()
                    .when(word_wrap, |d| d.whitespace_normal())
                    .child(text.clone())
                    .into_any_element()
            })
            .into_any_element()
    };

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
    message = message.footer(MessageFooter::new().child(super::message_footer::message_footer(mc, ws, md_state.clone(), cx)));
    let ws_menu = ws.clone();
    let ws_revert = ws.clone();
    let group = SharedString::from(format!("msg-{ix}"));
    div()
        .id(("msg", ix))
        .test_support()
        .group(group.clone())
        .child(message)
        // "Undo turn" on the user message that opened a checkpointed turn —
        // hover-revealed like the footer actions, hidden while a reply runs.
        .when(checkpointed, |d| {
            d.child(
                div().flex().justify_end().child(
                    div()
                        .id(("revert", ix))
                        .test_support()
                        .flex()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::Undo2)
                        .child("Undo turn")
                        .on_click(move |_, _, cx| {
                            ws_revert.update(cx, |this, cx| this.revert_to_checkpoint(ix, cx));
                        }),
                ),
            )
        })
        .context_menu(move |menu, _window, cx| {
            let ws_copy = ws_menu.clone();
            let ws_retry = ws_menu.clone();
            let ws_edit = ws_menu.clone();
            let ws_undo = ws_menu.clone();
            let menu = menu.item(
                gpui_kit::component::menu::PopupMenuItem::new("Copy")
                    .icon(IconName::Copy)
                    .on_click(move |_, _, cx| {
                        ws_copy.update(cx, |this, cx| this.copy_message(ix, cx));
                    }),
            );
            let menu = if let Some(md) = md_state.clone() {
                let label = if md.read(cx).raw { "View rendered" } else { "View raw" };
                menu.item(gpui_kit::component::menu::PopupMenuItem::new(label).icon(IconName::Code).on_click(
                    super::message_footer::toggle_raw(md, ws_menu.clone(), ix),
                ))
            } else if checkpointed {
                menu.item(gpui_kit::component::menu::PopupMenuItem::new("Undo turn").icon(IconName::Undo2).on_click(
                    move |_, _, cx| {
                        ws_undo.update(cx, |this, cx| this.revert_to_checkpoint(ix, cx));
                    },
                ))
            } else {
                menu
            };
            let menu =
                if role == Role::User {
                    menu.item(gpui_kit::component::menu::PopupMenuItem::new("Edit").icon(IconName::Pencil).on_click(
                        move |_, window, cx| {
                            ws_edit.update(cx, |this, cx| this.edit_message(ix, window, cx));
                        },
                    ))
                } else {
                    menu
                };
            if role == Role::Assistant && mc.is_last {
                menu.item(
                    gpui_kit::component::menu::PopupMenuItem::new("Retry")
                        .icon(IconName::RotateCcw)
                        .on_click(move |_, _, cx| {
                            ws_retry.update(cx, |this, cx| this.retry_last(cx));
                        }),
                )
            } else {
                menu
            }
        })
        .into_any_element()
}
