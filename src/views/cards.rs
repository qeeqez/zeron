use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{DiffCard, MessageKind, ToolCall, ToolStatus};
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
        .on_click(toggle_expanded(ws, ix));

    let mut card = card_frame(cx).child(header);
    if tool.expanded && !tool.output.is_empty() {
        card = card.child(detail_block(&tool.output, cx));
    }
    card
}

pub fn render_diff(ix: usize, diff: &DiffCard, ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let header = card_header(("diff", ix))
        .child(IconName::FileText)
        .child(diff.path.clone())
        .child(div().flex_1())
        .child(div().text_color(cx.theme().success).child(format!("+{}", diff.added)))
        .child(div().text_color(cx.theme().danger).child(format!("-{}", diff.removed)))
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(diff.expanded)))
        .on_click(toggle_expanded(ws, ix));

    let mut card = card_frame(cx).child(header);
    if diff.expanded {
        card = card.child(detail_block(&diff.hunks, cx));
    }
    card
}
