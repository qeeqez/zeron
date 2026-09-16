//! The ⋯ menu's "Compare providers…" item and its picker dialog — split
//! from `chat_menu.rs` for the SLOC cap (same pattern as
//! `chat_menu/continue_with.rs`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{Confirm, DialogClose, DialogFooter};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The "Compare providers…" row: opens the picker dialog. Disabled while
/// a reply runs, when there's nothing to send (empty transcript AND empty
/// composer draft — the draft is the prompt, the last user message the
/// fallback), or with fewer than two enabled providers.
pub(super) fn compare_item(menu: PopupMenu, ws: &Entity<Workspace>, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let this = ws.read(cx);
    let chat = &this.chats[this.active];
    let draft_empty = this.composer.read(cx).value().trim().is_empty();
    let off = chat.running || (chat.messages.is_empty() && draft_empty) || this.enabled_providers().len() < 2;
    let ws = ws.clone();
    menu.item(
        PopupMenuItem::new("Compare providers…")
            .icon(IconName::Columns3)
            .disabled(off)
            .on_click(move |_, window, cx| {
                ws.update(cx, |this, cx| this.open_compare_dialog(window, cx));
            }),
    )
}

impl Workspace {
    /// "Compare providers…" — a dialog listing every enabled provider
    /// instance with a checkbox; the chat's own provider starts checked.
    /// Compare needs two or more picks: each becomes a fork bound to that
    /// provider, and the same prompt is sent to all of them.
    pub fn open_compare_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let providers: Vec<CompareRow> = self
            .enabled_providers()
            .into_iter()
            .map(|p| CompareRow {
                id: p.id.clone(),
                name: p.name.clone(),
                icon: p.kind.info().icon,
                model: self.models_for(&p.id).first().map_or_else(String::new, |m| m.label.to_string()),
            })
            .collect();
        if providers.len() < 2 {
            return;
        }
        let chat = &self.chats[self.active];
        let current = if chat.provider.is_empty() { self.selected_provider.clone() } else { chat.provider.clone() };
        // Dialog-local state: the builder re-runs every render, so the set
        // lives in an Rc shared by the rows and the OK handler. The chat's
        // own provider starts checked — unless it isn't in the list (a
        // stamp on a since-disabled instance), which would inflate the
        // pick count with a row the user can't see.
        let picks = Rc::new(RefCell::new(HashSet::from_iter(providers.iter().any(|r| r.id == current).then_some(current))));
        let ws = cx.entity();
        // Fork order follows the dialog's list order — the first listed
        // pick ends up the active chat.
        let order: Vec<String> = providers.iter().map(|r| r.id.clone()).collect();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let count = picks.borrow().len();
            let ws_ok = ws.clone();
            let picks_ok = picks.clone();
            let order = order.clone();
            dialog
                .title("Compare providers")
                .overlay_closable(true)
                .w(px(380.))
                .child(
                    v_flex()
                        .id("compare-dialog")
                        .test_support()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Send the same prompt to each picked provider — one forked chat per pick."),
                        )
                        .children(providers.iter().map(|row| compare_row(row, &picks, cx))),
                )
                .footer(DialogFooter::new().child(DialogClose::new().trigger(|b| b.label("Cancel"))).child(
                    Button::new("compare-ok").primary().label("Compare").disabled(count < 2).on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
                    }),
                ))
                .on_ok(move |_, window, cx| confirm_compare(&ws_ok, &order, &picks_ok, window, cx))
        });
    }
}

/// Dialog OK: two or more picks fork the chat once per provider and send
/// each the same prompt; under two the dialog stays open (the Compare
/// button is already disabled — this guards the Enter path).
fn confirm_compare(
    ws: &Entity<Workspace>, order: &[String], picks: &Rc<RefCell<HashSet<String>>>, window: &mut Window, cx: &mut App,
) -> bool {
    let ids: Vec<String> = order.iter().filter(|id| picks.borrow().contains(*id)).cloned().collect();
    if ids.len() < 2 {
        return false;
    }
    ws.update(cx, |this, cx| this.compare_providers(&ids, window, cx));
    true
}

/// One dialog row's data — bundled so `compare_row` stays under the
/// arg-count lint.
struct CompareRow {
    id: String,
    name: String,
    icon: IconName,
    /// The fork's landing model (first effective) — shown muted.
    model: String,
}

/// One provider row: kind icon, a checkbox carrying the instance name, and
/// the model the fork would land on (its first effective model). Toggling
/// updates the shared pick set and repaints so Compare's disabled state
/// tracks the count.
fn compare_row(row: &CompareRow, picks: &Rc<RefCell<HashSet<String>>>, cx: &App) -> impl IntoElement {
    let pid = row.id.clone();
    let checked = picks.borrow().contains(&row.id);
    let picks = picks.clone();
    h_flex()
        .id(format!("compare-row-{pid}"))
        .test_support()
        .gap_2()
        .items_center()
        .child(row.icon)
        .child(
            Checkbox::new(format!("compare-check-{pid}"))
                .checked(checked)
                .label(row.name.clone())
                .on_click(move |on, _, cx| {
                    if *on {
                        picks.borrow_mut().insert(pid.clone());
                    } else {
                        picks.borrow_mut().remove(&pid);
                    }
                    cx.refresh_windows();
                }),
        )
        .when(!row.model.is_empty(), |r| {
            r.child(
                div()
                    .flex_1()
                    .text_right()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(row.model.clone()),
            )
        })
}
