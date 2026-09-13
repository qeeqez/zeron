use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, DiffCard, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

fn toggle_expanded(ws: Entity<Workspace>, ix: usize) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        ws.update(cx, |this, cx| {
            let Some(msg) = this.chats[this.active].messages.get_mut(ix) else { return };
            match &mut msg.kind {
                MessageKind::Tool(tool) => tool.expanded = !tool.expanded,
                MessageKind::Diff(diff) => diff.expanded = !diff.expanded,
                MessageKind::Text(_) => {},
            }
            // Height changed — the virtual scroller must re-measure or the
            // expanded body renders clipped. Under an open search the
            // scroller indexes the filtered list, so map real ix → position.
            let pos = this.filtered_pos(ix, cx);
            this.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
            cx.notify();
        });
    }
}

fn card_frame(cx: &App) -> Div {
    div().flex().flex_col().rounded_md().border_1().border_color(cx.theme().border).bg(cx.theme().muted)
}

fn card_header(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).flex().items_center().gap_2().px_3().py_2().cursor_pointer().text_sm()
}

fn detail_block(text: &SharedString, cx: &App) -> Div {
    div()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(cx.theme().muted_foreground)
        .child(text.clone())
}

fn chevron(expanded: bool) -> IconName {
    if expanded { IconName::ChevronDown } else { IconName::ChevronRight }
}

pub fn render_tool_call(ix: usize, tool: &ToolCall, ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let (icon, status_color) = match tool.status {
        ToolStatus::Running => (IconName::LoaderCircle, cx.theme().info),
        ToolStatus::Done => (IconName::CircleCheck, cx.theme().success),
        ToolStatus::Failed => (IconName::CircleX, cx.theme().danger),
    };

    let header = card_header(("tool", ix))
        .child(div().text_color(status_color).child(icon))
        .child(IconName::SquareTerminal)
        .child(tool.name.clone())
        .child(div().text_color(cx.theme().muted_foreground).child(tool.detail.clone()))
        .child(div().flex_1())
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(tool.expanded)))
        .on_click(toggle_expanded(ws.clone(), ix));

    let mut card = card_frame(cx).child(header);
    if tool.expanded && !tool.output.is_empty() {
        card = card.child(detail_block(&tool.output, cx)).child(tool_output_bar(ix, &ws, cx));
    }
    card
}

fn tool_output_bar(ix: usize, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let ws = ws.clone();
    div()
        .flex()
        .items_center()
        .justify_end()
        .px_3()
        .py_1()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .id(("copy-tool", ix))
                .cursor_pointer()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Copy output")
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.copy_message(ix, cx));
                }),
        )
}

pub fn render_diff(ix: usize, diff: &DiffCard, ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let header = card_header(("diff", ix))
        .child(IconName::FileText)
        .child(diff.path.clone())
        .child(div().flex_1())
        .child(div().text_color(cx.theme().success).child(format!("+{}", diff.added)))
        .child(div().text_color(cx.theme().danger).child(format!("-{}", diff.removed)))
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(diff.expanded)))
        .on_click(toggle_expanded(ws.clone(), ix));

    let mut card = card_frame(cx).child(header);
    if diff.expanded {
        card = card.child(detail_block(&diff.hunks, cx));
    }
    card
}

#[derive(Clone, Copy)]
pub struct MsgCtx {
    pub ix: usize,
    pub is_last: bool,
}

pub fn message_footer(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let MsgCtx { ix, is_last } = mc;
    let role = msg.role;
    let rating = msg.rating;
    let ws_copy = ws.clone();
    let ws_retry = ws.clone();
    let ws_regen = ws.clone();
    let ws_up = ws.clone();
    let ws_down = ws.clone();
    let muted = hsla(0.0, 0.0, 0.55, 1.0);
    let group = SharedString::from(format!("msg-{ix}"));
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .id(("copy", ix))
                .cursor_pointer()
                .invisible()
                .group_hover(group.clone(), |style| style.visible())
                .text_color(muted)
                .child(IconName::Copy)
                .on_click(move |_, _, cx| {
                    ws_copy.update(cx, |this, cx| this.copy_message(ix, cx));
                }),
        )
        .when(role == Role::Assistant, |d| {
            // retry_last re-runs the final turn — only meaningful on the
            // last message, so the icon is gated to it.
            d.when(is_last, |d| {
                d.child(
                    div()
                        .id(("retry", ix))
                        .cursor_pointer()
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .text_color(muted)
                        .child(IconName::RotateCcw)
                        .on_click(move |_, _, cx| {
                            ws_retry.update(cx, |this, cx| this.retry_last(cx));
                        }),
                )
                .child(
                    div()
                        .id(("regen", ix))
                        .cursor_pointer()
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .text_xs()
                        .text_color(muted)
                        .child("Regenerate")
                        .on_click(move |_, _, cx| {
                            ws_regen.update(cx, |this, cx| this.retry_last(cx));
                        }),
                )
            })
            .child(
                div()
                    .id(("up", ix))
                    .cursor_pointer()
                    .invisible()
                    .group_hover(group.clone(), |style| style.visible())
                    .child(IconName::ThumbsUp)
                    .text_color(if rating == Some(true) { cx.theme().accent } else { muted })
                    .on_click(move |_, _, cx| {
                        ws_up.update(cx, |this, cx| this.rate_message(ix, true, cx));
                    }),
            )
            .child(
                div()
                    .id(("down", ix))
                    .cursor_pointer()
                    .invisible()
                    .group_hover(group.clone(), |style| style.visible())
                    .child(IconName::ThumbsDown)
                    .text_color(if rating == Some(false) { cx.theme().accent } else { muted })
                    .on_click(move |_, _, cx| {
                        ws_down.update(cx, |this, cx| this.rate_message(ix, false, cx));
                    }),
            )
        })
        .child(div().flex_1())
        .when_some(msg.usage, |d, u| d.child(div().text_xs().text_color(muted).child(format!("{} in · {} out", u.input, u.output))))
        .child(div().text_xs().text_color(muted).child(format_time(msg.at)))
}

fn format_time(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at).format("%H:%M").to_string()
}
