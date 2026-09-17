use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, DiffCard, MessageKind, PlanCard, PlanStatus, ToolCall, ToolStatus};
use crate::workspace::Workspace;

/// A run of consecutive `Tool` messages — `head` is the first message's
/// index, `len` the run length. Runs of one aren't groups: `tool_group`
/// returns `None` for them so lone calls keep their plain card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToolGroup {
    pub head: usize,
    pub len: usize,
}

/// Group consecutive tool calls at render time: returns the run covering
/// message `ix` when it holds 2+ calls, else `None`. Any non-tool message —
/// text, plan, diff, approval — breaks the run.
pub(crate) fn tool_group(messages: &[ChatMessage], ix: usize) -> Option<ToolGroup> {
    if !matches!(messages.get(ix)?.kind, MessageKind::Tool(_)) {
        return None;
    }
    let mut head = ix;
    while head > 0 && matches!(messages[head - 1].kind, MessageKind::Tool(_)) {
        head -= 1;
    }
    let mut end = ix + 1;
    while end < messages.len() && matches!(messages[end].kind, MessageKind::Tool(_)) {
        end += 1;
    }
    (end - head > 1).then_some(ToolGroup { head, len: end - head })
}

/// Flip a group's collapsed state and re-measure every row it covers —
/// expanding reveals the member rows below the summary, collapsing hides
/// them again.
fn toggle_tool_group(ws: Entity<Workspace>, g: ToolGroup) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        ws.update(cx, |this, cx| {
            let chat = &mut this.chats[this.active];
            let Some(head) = chat.messages.get(g.head) else { return };
            let key = (g.head, head.at);
            if !chat.expanded_tool_groups.remove(&key) {
                chat.expanded_tool_groups.insert(key);
            }
            let start = this.filtered_pos(g.head, cx);
            this.scroller.update(cx, |s, cx| s.remeasure_items(start..start + g.len, cx));
            cx.notify();
        });
    }
}

/// Header + optional detail of one tool call — shared by the standalone
/// card and the group's head row, which embeds its first call unframed.
fn tool_card_body(ix: usize, tool: &ToolCall, ws: &Entity<Workspace>, cx: &mut App) -> Div {
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
        .on_click(toggle_expanded(ws.clone(), ix))
        .test_support();

    let mut body = div().flex().flex_col().child(header);
    if tool.expanded && !tool.output.is_empty() {
        body = body.child(detail_block(&tool.output, cx)).child(tool_output_bar(ix, ws, cx));
    }
    body
}

pub fn render_tool_call(ix: usize, tool: &ToolCall, ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    card_frame(cx).child(tool_card_body(ix, tool, &ws, cx))
}

/// Collapsed summary for a run of tool calls: "N tool calls" plus the tool
/// names, spinning while any member runs. Expanding reveals the run's own
/// calls — the head's card embeds here, the rest render in their rows.
pub fn render_tool_group(g: ToolGroup, messages: &[ChatMessage], expanded: bool, ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let members = &messages[g.head..g.head + g.len];
    let names = members
        .iter()
        .filter_map(|m| match &m.kind {
            MessageKind::Tool(t) => Some(t.name.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let running = members.iter().any(|m| matches!(&m.kind, MessageKind::Tool(t) if t.status == ToolStatus::Running));
    let status = if running {
        (IconName::LoaderCircle, cx.theme().info)
    } else {
        (IconName::CircleCheck, cx.theme().success)
    };

    let header = card_header(("tool-group", g.head))
        .aria_label(format!("{} tool calls", g.len))
        .child(div().text_color(status.1).child(status.0))
        .child(IconName::SquareTerminal)
        .child(format!("{} tool calls", g.len))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(cx.theme().muted_foreground)
                .child(names),
        )
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(expanded)))
        .on_click(toggle_tool_group(ws.clone(), g))
        .test_support();

    let mut card = card_frame(cx).child(header);
    if expanded && let MessageKind::Tool(tool) = &members[0].kind {
        card = card.child(div().border_t_1().border_color(cx.theme().border).child(tool_card_body(g.head, tool, &ws, cx)));
    }
    card
}

fn toggle_expanded(ws: Entity<Workspace>, ix: usize) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        ws.update(cx, |this, cx| {
            let Some(msg) = std::rc::Rc::make_mut(&mut this.chats[this.active].messages).get_mut(ix) else { return };
            match &mut msg.kind {
                MessageKind::Tool(tool) => tool.expanded = !tool.expanded,
                MessageKind::Diff(diff) => diff.expanded = !diff.expanded,
                // Approval cards always show their detail — there's
                // nothing to collapse.
                MessageKind::Text(_) | MessageKind::Plan(_) | MessageKind::Approval(_) => {},
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

pub(crate) fn card_frame(cx: &App) -> Div {
    div().flex().flex_col().rounded_md().border_1().border_color(cx.theme().border).bg(cx.theme().muted)
}

pub(crate) fn card_header(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).flex().items_center().gap_2().px_3().py_2().cursor_pointer().text_sm()
}

pub(crate) fn detail_block(text: &SharedString, cx: &App) -> Div {
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

pub(crate) fn chevron(expanded: bool) -> IconName {
    if expanded { IconName::ChevronDown } else { IconName::ChevronRight }
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
                .test_support()
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

/// The agent's plan checklist — one row per step, live-updating as `Plan`
/// events stream in. Done steps strike through, the in-progress step is
/// highlighted, pending steps stay dim.
pub fn render_plan(ix: usize, plan: &PlanCard, cx: &mut App) -> impl IntoElement {
    let done = plan.steps.iter().filter(|s| s.status == PlanStatus::Done).count();
    let header = div()
        .id(("plan", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .text_sm()
        .child(IconName::ListTodo)
        .child("Plan")
        .child(div().flex_1())
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{done}/{}", plan.steps.len())),
        );
    let steps: Vec<AnyElement> = plan.steps.iter().map(|s| plan_step(ix, s, cx).into_any_element()).collect();
    card_frame(cx).child(header).children(steps)
}

/// One checklist row: a status checkbox icon plus the step label. The row
/// carries `Role::CheckBox` + `aria_toggled` so tests and a11y clients see
/// done/in-progress/pending as checked/mixed/unchecked.
fn plan_step(ix: usize, step: &crate::model::PlanStep, cx: &mut App) -> impl IntoElement {
    use gpui_kit::accesskit::Toggled;
    let (icon, color, toggled) = match step.status {
        PlanStatus::Done => (IconName::SquareCheck, cx.theme().success, Toggled::True),
        PlanStatus::InProgress => (IconName::LoaderCircle, cx.theme().info, Toggled::Mixed),
        PlanStatus::Pending => (IconName::Square, cx.theme().muted_foreground, Toggled::False),
    };
    div()
        .id(format!("plan-step-{ix}-{}", step.id))
        .test_support()
        .role(gpui_kit::Role::CheckBox)
        .aria_toggled(toggled)
        .aria_label(step.label.clone())
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(div().text_color(color).child(icon))
        .child(
            div()
                .text_sm()
                .when(step.status == PlanStatus::Done, |d| d.line_through().text_color(cx.theme().muted_foreground))
                .when(step.status == PlanStatus::InProgress, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(step.label.clone()),
        )
}

#[derive(Clone, Copy)]
pub struct MsgCtx<'a> {
    pub ix: usize,
    pub is_last: bool,
    /// Duration of the completed turn — set only on the last real message
    /// once the turn is done; drives the "Worked for Ns" label.
    pub duration: Option<std::time::Duration>,
    pub msg: &'a ChatMessage,
}
