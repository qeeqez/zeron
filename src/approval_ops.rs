//! Approval-prompt answers: the card's click path plus the deny-on-stop
//! fallback that unblocks a backend whose turn was cancelled.

use std::rc::Rc;

use gpui_kit::*;

use crate::model::MessageKind;
use crate::workspace::Workspace;

impl Workspace {
    /// Answer the approval prompt on message `ix` of the active chat:
    /// record the decision on the card and send it to the backend thread
    /// blocked on the request. A missing responder means the prompt
    /// expired — the card still records the click.
    pub fn answer_approval(&mut self, ix: usize, decision: crate::backend::ApprovalDecision, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        let Some(msg) = Rc::make_mut(&mut chat.messages).get_mut(ix) else { return };
        let MessageKind::Approval(card) = &mut msg.kind else { return };
        if card.decision.is_some() {
            return;
        }
        card.decision = Some(decision);
        if let Some(respond) = card.respond.take() {
            let _ = respond.send(decision);
        }
        // Buttons collapse into the outcome line — the virtual scroller
        // must re-measure or the card renders at its old height.
        let pos = self.filtered_pos(ix, cx);
        self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        cx.notify();
    }
}

/// Record Deny on a pending approval card and answer the backend's blocked
/// read so a stopped turn can't hang waiting on a click that won't come.
pub(crate) fn deny_approval(a: &mut crate::backend::ApprovalCard) {
    a.decision = Some(crate::backend::ApprovalDecision::Deny);
    if let Some(respond) = a.respond.take() {
        let _ = respond.send(crate::backend::ApprovalDecision::Deny);
    }
}
