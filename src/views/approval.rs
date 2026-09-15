//! The approval-prompt card: what the backend wants to run plus
//! Approve / Deny / Always-allow buttons while it waits, or the recorded
//! outcome once answered.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::{ApprovalDecision, ApprovalKind};
use crate::backend::ApprovalCard;
use crate::views::cards::{card_frame, detail_block};
use crate::workspace::Workspace;

/// An approval prompt: the gated action's detail plus the decision buttons
/// while the backend waits, or the recorded outcome once answered. A card
/// whose responder is gone (turn stopped, chat reloaded) shows as expired
/// instead of offering dead buttons.
pub fn render_approval(ix: usize, card: &ApprovalCard, ws: Entity<Workspace>, cx: &mut App) -> impl IntoElement {
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
        card_el = card_el.child(detail_block(&card.detail, cx).id(("approval-detail", ix)).test_support());
    }
    card_el.child(approval_footer(ix, card, &ws, cx)).id(("approval", ix)).test_support()
}

/// The card's bottom row: live buttons while the backend waits on this
/// prompt, the recorded decision once answered, or an expired note when
/// the responder is gone without a decision.
fn approval_footer(ix: usize, card: &ApprovalCard, ws: &Entity<Workspace>, cx: &mut App) -> Div {
    let bar = div()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border);
    if let Some(decision) = card.decision {
        let color = match decision {
            ApprovalDecision::Deny => cx.theme().danger,
            _ => cx.theme().success,
        };
        return bar.child(div().id(("approval-outcome", ix)).test_support().text_xs().text_color(color).child(decision.label()));
    }
    if card.respond.is_none() {
        return bar.child(
            div().id(("approval-outcome", ix)).test_support().text_xs().text_color(cx.theme().muted_foreground).child("Approval expired"),
        );
    }
    bar.child(approval_button(Btn::new(("deny", ix), ix, "Deny", ApprovalDecision::Deny), ws, cx))
        .child(approval_button(Btn::new(("always", ix), ix, "Always allow", ApprovalDecision::ApproveForSession), ws, cx))
        .child(approval_button(Btn::new(("approve", ix), ix, "Approve", ApprovalDecision::Approve), ws, cx))
}

/// One decision button's identity: element id, message index the answer
/// targets, label, and the decision a click sends.
struct Btn {
    id: ElementId,
    ix: usize,
    label: &'static str,
    decision: ApprovalDecision,
}

impl Btn {
    fn new(id: impl Into<ElementId>, ix: usize, label: &'static str, decision: ApprovalDecision) -> Self {
        Self { id: id.into(), ix, label, decision }
    }
}

/// One decision button — sends the choice through `answer_approval`.
fn approval_button(btn: Btn, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let (bg, fg) = match btn.decision {
        ApprovalDecision::Approve => (cx.theme().accent, cx.theme().accent_foreground),
        ApprovalDecision::Deny => (cx.theme().danger, cx.theme().accent_foreground),
        ApprovalDecision::ApproveForSession => (cx.theme().muted, cx.theme().foreground),
    };
    div()
        .id(btn.id)
        .test_support()
        .cursor_pointer()
        .px_3()
        .py_1()
        .rounded_md()
        .text_xs()
        .bg(bg)
        .text_color(fg)
        .child(btn.label)
        .on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| this.answer_approval(btn.ix, btn.decision, cx));
        })
}
