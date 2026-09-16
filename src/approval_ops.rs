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
        // "Always allow" also records a durable rule so later matching
        // requests auto-approve — the session grant alone dies with the
        // turn. `card` borrows the chat, so the rule is built before the
        // `&mut self` call.
        let rule = (decision == crate::backend::ApprovalDecision::ApproveForSession)
            .then(|| crate::backend::ApprovalRule::for_prompt(card.kind, &card.detail));
        if let Some(rule) = rule {
            self.allow_rule(rule);
        }
        // Buttons collapse into the outcome line — the virtual scroller
        // must re-measure or the card renders at its old height.
        let pos = self.filtered_pos(ix, cx);
        self.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        cx.notify();
    }

    /// Does a stored rule cover this prompt? `detail` normalizes the same
    /// way `for_prompt` does, so a click and a later request agree.
    pub(crate) fn approval_rule_allows(&self, kind: crate::backend::ApprovalKind, detail: &str) -> bool {
        let rule = crate::backend::ApprovalRule::for_prompt(kind, detail);
        (!rule.detail.is_empty()) && self.approval_rules.contains(&rule)
    }

    /// Record a durable allow rule (deduped) and persist it to the
    /// project's `state.json`.
    pub(crate) fn allow_rule(&mut self, rule: crate::backend::ApprovalRule) {
        if rule.detail.is_empty() || self.approval_rules.contains(&rule) {
            return;
        }
        self.approval_rules.push(rule);
        self.save_project_state();
    }

    /// The Settings → Project rules list's Delete: drop rule `ix` and
    /// persist. Later matching requests prompt again.
    pub fn remove_approval_rule(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.approval_rules.len() {
            self.approval_rules.remove(ix);
            self.save_project_state();
            cx.notify();
        }
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

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "approval_allowlist_tests.rs"]
mod approval_allowlist_tests;
