use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{MessageKind, Role};

use crate::views::cards::{MsgCtx, render_diff, render_tool_call};
use crate::views::markdown::MarkdownState;
use crate::workspace::Workspace;

pub fn render_message(mc: MsgCtx, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    match &msg.kind {
        MessageKind::Text(_) => render_text(mc, ws, window, cx),
        MessageKind::Tool(tool) => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
    }
}

fn render_text(mc: MsgCtx, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let role = msg.role;
    let word_wrap = ws.read(cx).word_wrap;
    let font_size = ws.read(cx).font_size;
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
    let body = div()
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
        .child(if let Some(state) = &md_state {
            super::markdown::assistant_markdown(ix, text, state, cx)
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
    message = message.footer(MessageFooter::new().child(message_footer(mc, ws, md_state.clone(), cx)));
    let ws_menu = ws.clone();
    div()
        .id(("msg", ix))
        .test_support()
        .group(SharedString::from(format!("msg-{ix}")))
        .child(message)
        .context_menu(move |menu, _window, cx| {
            let ws_copy = ws_menu.clone();
            let ws_retry = ws_menu.clone();
            let ws_edit = ws_menu.clone();
            let menu = menu.item(
                gpui_kit::component::menu::PopupMenuItem::new("Copy")
                    .icon(IconName::Copy)
                    .on_click(move |_, _, cx| {
                        ws_copy.update(cx, |this, cx| this.copy_message(ix, cx));
                    }),
            );
            let menu = if let Some(md) = md_state.clone() {
                let label = if md.read(cx).raw { "View rendered" } else { "View raw" };
                menu.item(gpui_kit::component::menu::PopupMenuItem::new(label).icon(IconName::Code).on_click(toggle_raw(
                    md,
                    ws_menu.clone(),
                    ix,
                )))
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

/// Flip message `ix` between rendered Markdown and its source. The flag
/// lives on the keyed `MarkdownState`; the row's height changes, so the
/// virtual scroller re-measures it (filtered position, like expand).
fn toggle_raw(state: Entity<MarkdownState>, ws: Entity<Workspace>, ix: usize) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        state.update(cx, |md, cx| {
            md.raw = !md.raw;
            cx.notify();
        });
        ws.update(cx, |this, cx| {
            let pos = this.filtered_pos(ix, cx);
            this.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        });
    }
}

/// One ghost icon in the hover toolbar — invisible until the `msg-{ix}`
/// group is hovered.
fn action_icon(
    id: (&'static str, usize), icon: IconName, color: Hsla, group: &SharedString,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .cursor_pointer()
        .invisible()
        .group_hover(group.clone(), |style| style.visible())
        .text_color(color)
        .child(icon)
        .on_click(on_click)
}

/// Hover-revealed action row under a message: copy, view-raw, retry (last
/// assistant reply only), rating, read-aloud, then the turn duration,
/// token usage and timestamp.
fn message_footer(mc: MsgCtx, ws: &Entity<Workspace>, md_state: Option<Entity<MarkdownState>>, cx: &mut App) -> Div {
    let MsgCtx { ix, is_last, msg, .. } = mc;
    let muted = hsla(0.0, 0.0, 0.55, 1.0);
    let accent = cx.theme().accent;
    let group = SharedString::from(format!("msg-{ix}"));
    let mut row = div().flex().items_center().gap_1();
    {
        let ws = ws.clone();
        row = row.child(action_icon(("copy", ix), IconName::Copy, muted, &group, move |_, _, cx| {
            ws.update(cx, |this, cx| this.copy_message(ix, cx));
        }));
    }
    if msg.role == Role::Assistant {
        // View-raw flips the body between rendered Markdown and source.
        if let Some(md) = md_state {
            let color = if md.read(cx).raw { accent } else { muted };
            row = row.child(action_icon(("raw", ix), IconName::Code, color, &group, toggle_raw(md, ws.clone(), ix)));
        }
        // retry_last re-runs the final turn — only meaningful on the last
        // message, so the icon is gated to it.
        if is_last {
            let ws = ws.clone();
            row = row.child(action_icon(("retry", ix), IconName::RotateCcw, muted, &group, move |_, _, cx| {
                ws.update(cx, |this, cx| this.retry_last(cx));
            }));
        }
        let rating = msg.rating;
        for (id, icon, up) in [("up", IconName::ThumbsUp, true), ("down", IconName::ThumbsDown, false)] {
            let ws = ws.clone();
            let color = if rating == Some(up) { accent } else { muted };
            row = row.child(action_icon((id, ix), icon, color, &group, move |_, _, cx| {
                ws.update(cx, |this, cx| this.rate_message(ix, up, cx));
            }));
        }
        {
            let ws = ws.clone();
            row = row.child(action_icon(("speak", ix), IconName::Volume2, muted, &group, move |_, _, cx| {
                ws.update(cx, |this, _cx| this.speak_message(ix));
            }));
        }
        if let Some(dur) = mc.duration {
            row = row.child(
                div()
                    .id(("worked", ix))
                    .test_support()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("Worked for {}s", dur.as_secs())),
            );
        }
    }
    row.child(div().flex_1())
        .when_some(msg.usage, |d, u| d.child(div().text_xs().text_color(muted).child(format!("{} in · {} out", u.input, u.output))))
        .child(div().text_xs().text_color(muted).child(format_time(msg.at)))
}

fn format_time(at: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at).format("%H:%M").to_string()
}
