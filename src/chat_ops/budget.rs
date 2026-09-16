//! Per-chat spend caps — a chat's accumulated cost (the same estimate the
//! usage popover shows) is checked against an effective cap when each turn
//! completes: the chat's own `budget_alert_usd` override, else the global
//! `Workspace::budget_alert_usd` default. Crossing the cap raises a
//! dismissible banner under the transcript plus a transcript note. The
//! alert records the cap it fired under, so raising the cap re-arms it —
//! the banner hides until spend crosses the new cap.

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// The cap a chat's spend is checked against — its own override when
    /// set, else the global default. `None` = uncapped.
    pub(crate) fn budget_cap(&self, chat: &crate::model::Chat) -> Option<f64> {
        chat.budget_alert_usd.or(self.budget_alert_usd)
    }

    /// The chat's estimated spend — the same total the usage popover
    /// prices: cumulative tokens under the chat's model (the live
    /// selection stands in for legacy chats). `None` = unpriced model.
    pub(crate) fn chat_cost(&self, chat: &crate::model::Chat) -> Option<f64> {
        let model = if chat.model.is_empty() { self.model.as_ref() } else { chat.model.as_str() };
        chat.usage.cost(model)
    }

    /// `(spend, cap)` while the banner should show — the alert fired under
    /// the *current* effective cap and wasn't dismissed. A cap change
    /// hides a stale alert until spend crosses the new cap.
    pub(crate) fn budget_alert_visible(&self, chat: &crate::model::Chat) -> Option<(f64, f64)> {
        if chat.budget_dismissed {
            return None;
        }
        let cap = self.budget_cap(chat)?;
        if chat.budget_alerted != Some(cap) {
            return None;
        }
        Some((self.chat_cost(chat)?, cap))
    }

    /// Turn-end check: raise the banner + note when the chat's spend
    /// crosses the effective cap. `budget_alerted` records the cap it
    /// fired under — a changed cap re-arms the alert.
    pub(crate) fn check_budget_alert(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let Some(cap) = self.budget_cap(chat) else { return };
        let Some(cost) = self.chat_cost(chat) else { return };
        if cost < cap || chat.budget_alerted == Some(cap) {
            return;
        }
        let text =
            format!("**Budget alert:** this chat has spent ~{} (cap {}).", crate::pricing::fmt_cost(cost), crate::pricing::fmt_cost(cap));
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else { return };
        chat.budget_alerted = Some(cap);
        chat.budget_dismissed = false;
        self.note_in(chat_id, text, cx);
    }

    /// The banner's Dismiss — hides the alert until the cap changes.
    pub(crate) fn dismiss_budget_alert(&mut self, cx: &mut Context<Self>) {
        self.chats[self.active].budget_dismissed = true;
        cx.notify();
    }

    /// Set the chat's cap override; `None` clears it back to the global
    /// default. Persisted like the other per-chat fields; a cap lowered
    /// under the current spend alerts immediately rather than waiting for
    /// the next turn.
    pub(crate) fn set_chat_budget(&mut self, id: u64, cap: Option<f64>, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == id) else { return };
        if chat.budget_alert_usd == cap {
            return;
        }
        chat.budget_alert_usd = cap;
        cx.notify();
        self.save();
        self.check_budget_alert(id, cx);
    }

    /// Set the global default cap (`Settings.budget_alert_usd`) and
    /// persist it. Chats riding the default re-check now — a lowered cap
    /// alerts on the next render, not the next turn.
    pub(crate) fn set_budget_alert_usd(&mut self, cap: Option<f64>, cx: &mut Context<Self>) {
        if self.budget_alert_usd == cap {
            return;
        }
        self.budget_alert_usd = cap;
        self.save_settings();
        let ids: Vec<u64> = self.chats.iter().filter(|c| c.budget_alert_usd.is_none()).map(|c| c.id).collect();
        for id in ids {
            self.check_budget_alert(id, cx);
        }
        cx.notify();
    }

    /// "Budget alert…" from the ⋯ menu — a small dialog seeded with the
    /// chat's override; OK saves, empty input clears back to the global
    /// default. Shares `Workspace::budget_input` like the folder dialogs
    /// share `folder_input`.
    pub fn open_chat_budget(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == id) else { return };
        let current = chat.budget_alert_usd.map(|c| c.to_string()).unwrap_or_default();
        self.budget_input.update(cx, |state, cx| state.set_value(current, window, cx));
        let ws = cx.entity();
        let input = self.budget_input.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ws_ok = ws.clone();
            dialog
                .title("Budget alert")
                .overlay_closable(true)
                .child(Input::new(&input))
                .on_ok(move |_, _window, cx| ws_ok.update(cx, |this, cx| this.commit_chat_budget(id, cx)))
        });
        self.budget_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Budget alert" — empty input clears the chat's
    /// override; invalid input keeps the dialog open.
    fn commit_chat_budget(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        let text = self.budget_input.read(cx).value().to_string();
        match parse_budget(&text) {
            Ok(cap) => {
                self.set_chat_budget(id, cap, cx);
                true
            },
            Err(()) => false,
        }
    }

    /// The General section's budget field commit (Enter or blur): parse
    /// the draft into the global default; an invalid draft snaps back to
    /// the stored value.
    pub(crate) fn commit_budget_cap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.budget_cap_input.read(cx).value().to_string();
        if let Ok(cap) = parse_budget(&draft) {
            self.set_budget_alert_usd(cap, cx);
        }
        let stored = self.budget_alert_usd.map(|c| c.to_string()).unwrap_or_default();
        self.budget_cap_input.update(cx, |input, cx| input.set_value(stored, window, cx));
        cx.notify();
    }
}

/// A dollar amount: empty = no cap, a positive number = the cap. `0` and
/// negatives count as no cap — a zero cap would fire on the first token.
fn parse_budget(text: &str) -> Result<Option<f64>, ()> {
    let text = text.trim().trim_start_matches('$').trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0. => Ok(Some(v)),
        Ok(_) => Ok(None),
        Err(_) => Err(()),
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "../budget_tests.rs"]
mod budget_tests;
#[cfg(test)]
#[path = "../budget_ui_tests.rs"]
mod budget_ui_tests;
