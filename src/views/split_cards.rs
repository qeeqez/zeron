//! The split pane's card renderers — tool calls, diffs and approvals in
//! their read-only form. Expand toggles are the pane's only interactive
//! elements: they flip view state on the *secondary* chat and re-measure
//! this pane's scroller, never the active one. Approval cards lose their
//! buttons — the decision belongs to the active pane.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::{ApprovalCard, ApprovalDecision, ApprovalKind};
use crate::model::{DiffCard, MessageKind, ToolCall, ToolStatus};
use crate::views::cards::{card_frame, card_header, chevron, detail_block};
use crate::workspace::Workspace;

/// A tool call card with a working expand toggle.
pub(super) fn split_tool(ix: usize, tool: &ToolCall, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let (icon, status_color) = match tool.status {
        ToolStatus::Running => (IconName::LoaderCircle, cx.theme().info),
        ToolStatus::Done => (IconName::CircleCheck, cx.theme().success),
        ToolStatus::Failed => (IconName::CircleX, cx.theme().danger),
    };
    let header = card_header(("split-tool", ix))
        .child(div().text_color(status_color).child(icon))
        .child(IconName::SquareTerminal)
        .child(tool.name.clone())
        .child(div().text_color(cx.theme().muted_foreground).child(tool.detail.clone()))
        .child(div().flex_1())
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(tool.expanded)))
        .on_click(split_toggle_expanded(ix, ws.clone()))
        .test_support();
    let mut body = div().flex().flex_col().child(header);
    if tool.expanded && !tool.output.is_empty() {
        body = body.child(detail_block(&tool.output, cx));
    }
    card_frame(cx).child(body)
}

/// A diff card — same expand toggle as the tool card.
pub(super) fn split_diff(ix: usize, diff: &DiffCard, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let header = card_header(("split-diff", ix))
        .child(IconName::FileText)
        .child(diff.path.clone())
        .child(div().flex_1())
        .child(div().text_color(cx.theme().success).child(format!("+{}", diff.added)))
        .child(div().text_color(cx.theme().danger).child(format!("-{}", diff.removed)))
        .child(div().text_color(cx.theme().muted_foreground).child(chevron(diff.expanded)))
        .on_click(split_toggle_expanded(ix, ws.clone()))
        .test_support();
    let mut card = card_frame(cx).child(header);
    if diff.expanded {
        card = card.child(detail_block(&diff.hunks, cx));
    }
    card
}

/// Flip `expanded` on message `ix` of the *secondary* chat and re-measure
/// the split scroller's row. A stale click (pane swapped or closed since
/// render) resolves to nothing rather than touching the active chat.
fn split_toggle_expanded(ix: usize, ws: Entity<Workspace>) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        ws.update(cx, |this, cx| {
            let Some(six) = this.secondary else { return };
            let Some(chat) = this.chats.get_mut(six) else { return };
            let Some(msg) = Rc::make_mut(&mut chat.messages).get_mut(ix) else { return };
            match &mut msg.kind {
                MessageKind::Tool(tool) => tool.expanded = !tool.expanded,
                MessageKind::Diff(diff) => diff.expanded = !diff.expanded,
                MessageKind::Text(_) | MessageKind::Plan(_) | MessageKind::Approval(_) => {},
            }
            this.secondary_scroller.update(cx, |s, cx| s.remeasure_items(ix..ix + 1, cx));
            cx.notify();
        });
    }
}

/// An approval card without its buttons — a live prompt shows as awaiting
/// instead of offering buttons that would answer the wrong chat.
pub(super) fn split_approval(ix: usize, card: &ApprovalCard, cx: &mut App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let icon = match card.kind {
        ApprovalKind::Command => IconName::SquareTerminal,
        ApprovalKind::Patch => IconName::FileText,
        ApprovalKind::Permission => IconName::ShieldAlert,
    };
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .text_sm()
        .child(div().text_color(cx.theme().warning).child(icon))
        .child(card.kind.label())
        .child(div().flex_1());
    let mut card_el = card_frame(cx).child(header);
    if !card.detail.is_empty() {
        card_el = card_el.child(detail_block(&card.detail, cx));
    }
    let (label, color) = match card.decision {
        Some(_) if card.auto_approved => ("Auto-approved · rule", cx.theme().success),
        Some(d) => (d.label(), if d == ApprovalDecision::Deny { cx.theme().danger } else { cx.theme().success }),
        None if card.respond.is_none() => ("Approval expired", cx.theme().muted_foreground),
        None => ("Awaiting approval", cx.theme().muted_foreground),
    };
    card_el
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(div().text_xs().text_color(color).child(label)),
        )
        .id(("split-approval", ix))
        .test_support()
}
