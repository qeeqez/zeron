//! The Plan panel's state and the "current plan" extraction — split from
//! `workspace.rs` for the SLOC cap. The panel itself renders in
//! `views::plan_panel`; this file owns what it shows: the active chat's
//! latest `PlanCard` (the last `MessageKind::Plan` in transcript order —
//! `apply_plan` rewrites a card's steps in place, so position is stable)
//! plus the open flag persisted via `Settings.plan_panel_open`.

use gpui_kit::Context;

use crate::model::{Chat, MessageKind, PlanCard, PlanStatus};
use crate::workspace::Workspace;

/// Plan-panel state — just the open flag today; kept as a struct (like
/// `SnapshotsState`/`TerminalPanel`) so panel state has one home.
#[derive(Default)]
pub struct PlanPanel {
    /// Whether the side panel is mounted — persisted as
    /// `Settings.plan_panel_open`, toggled by `toggle_plan_panel`.
    pub open: bool,
}

impl Chat {
    /// The chat's current plan: the last plan card in transcript order.
    /// `None` when the agent never called `update_plan` this thread.
    pub fn latest_plan(&self) -> Option<&PlanCard> {
        self.messages.iter().rev().find_map(|m| match &m.kind {
            MessageKind::Plan(p) => Some(p),
            _ => None,
        })
    }
}

impl PlanCard {
    /// Steps marked done — the "3/5" progress count's numerator.
    pub fn done_count(&self) -> usize {
        self.steps.iter().filter(|s| s.status == PlanStatus::Done).count()
    }
}

impl Workspace {
    /// The active chat's current plan — what the panel lists.
    pub fn active_plan(&self) -> Option<&PlanCard> {
        self.chats.get(self.active).and_then(Chat::latest_plan)
    }

    /// Toggle the Plan panel; the open flag persists like the terminal's.
    pub fn toggle_plan_panel(&mut self, cx: &mut Context<Self>) {
        self.plan_panel.open = !self.plan_panel.open;
        self.save_settings();
        cx.notify();
    }
}
