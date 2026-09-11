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
    let status = match diff.applied {
        Some(true) => Some(("Applied", cx.theme().success)),
        Some(false) => Some(("Rejected", cx.theme().danger)),
        None => None,
    };
    let header = card_header(("diff", ix))
        .child(IconName::FileText)
        .child(diff.path.clone())
        .child(div().flex_1())
        .when_some(status, |d, (label, color)| d.child(div().text_xs().text_color(color).child(label)))
        .child(div().text_color(cx.theme().success).child(format!("+{}", diff.added)))
        .child(div().text_color(cx.theme().danger).child(format!("-{}", diff.removed)))
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(diff.expanded)))
        .on_click(toggle_expanded(ws.clone(), ix));

    let mut card = card_frame(cx).child(header);
    if diff.expanded {
        card = card.child(detail_block(&diff.hunks, cx));
    }
    if diff.applied.is_none() {
        card = card.child(diff_actions(ix, &ws, cx));
    }
    card
}

fn diff_actions(ix: usize, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let ws_apply = ws.clone();
    let ws_reject = ws.clone();
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .id(("apply", ix))
                .cursor_pointer()
                .text_xs()
                .text_color(cx.theme().success)
                .child("Apply")
                .on_click(move |_, _, cx| {
                    ws_apply.update(cx, |this, cx| this.set_diff_applied(ix, true, cx));
                }),
        )
        .child(
            div()
                .id(("reject", ix))
                .cursor_pointer()
                .text_xs()
                .text_color(cx.theme().danger)
                .child("Reject")
                .on_click(move |_, _, cx| {
                    ws_reject.update(cx, |this, cx| this.set_diff_applied(ix, false, cx));
                }),
        )
}

pub fn message_footer(ix: usize, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let role = msg.role;
    let rating = msg.rating;
    let ws_copy = ws.clone();
    let ws_retry = ws.clone();
    let ws_up = ws.clone();
    let ws_down = ws.clone();
    let muted = hsla(0.0, 0.0, 0.55, 1.0);
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .id(("copy", ix))
                .cursor_pointer()
                .text_color(muted)
                .child(IconName::Copy)
                .on_click(move |_, _, cx| {
                    ws_copy.update(cx, |this, cx| this.copy_message(ix, cx));
                }),
        )
        .when(role == Role::Assistant, |d| {
            d.child(
                div()
                    .id(("retry", ix))
                    .cursor_pointer()
                    .text_color(muted)
                    .child(IconName::RotateCcw)
                    .on_click(move |_, _, cx| {
                        ws_retry.update(cx, |this, cx| this.retry_last(cx));
                    }),
            )
            .child(
                div()
                    .id(("up", ix))
                    .cursor_pointer()
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
                    .child(IconName::ThumbsDown)
                    .text_color(if rating == Some(false) { cx.theme().accent } else { muted })
                    .on_click(move |_, _, cx| {
                        ws_down.update(cx, |this, cx| this.rate_message(ix, false, cx));
                    }),
            )
        })
}
