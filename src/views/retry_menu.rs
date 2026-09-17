//! The message context menu's retry tail: "Retry" re-runs the last turn
//! as-is; "Regenerate with model" lists every enabled instance's models —
//! the same effective lists the composer picker shows — checked on the
//! live selection. A pick switches the selection (`select_model`, the
//! picker's own write path) and re-runs the turn; picking the current
//! model is a plain retry. Fewer than two switchable models hides the
//! submenu — "Retry" alone covers that case.
//!
//! Declared in `views::mod`; the SLOC cap keeps it out of `message.rs`.
use gpui_kit::assets::IconName;
use gpui_kit::component::Side;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::workspace::Workspace;

/// One switchable model: the instance it belongs to, its id, the
/// "{provider} · {model}" label, the kind icon, and whether it's the live
/// selection.
struct ModelPick {
    provider: String,
    id: SharedString,
    label: String,
    icon: IconName,
    current: bool,
}

/// Every enabled instance's effective models, in picker order — the
/// workspace's `available_models` with the menu-facing fields attached.
fn model_picks(ws: &Entity<Workspace>, cx: &App) -> Vec<ModelPick> {
    let this = ws.read(cx);
    this.available_models()
        .into_iter()
        .map(|(p, m)| ModelPick {
            provider: p.id.clone(),
            id: m.id.clone(),
            label: format!("{} · {}", p.name, m.label),
            icon: p.kind.info().icon,
            current: this.selected_provider == p.id && this.model == m.id,
        })
        .collect()
}

/// The retry tail of a message's context menu — call only on the last
/// assistant message, where `retry_last` is meaningful.
pub(crate) fn retry_items(menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let ws_retry = ws.clone();
    let menu = menu.item(PopupMenuItem::new("Retry").icon(IconName::RotateCcw).on_click(move |_, _w, cx| {
        ws_retry.update(cx, |this, cx| this.retry_last(cx));
    }));
    retry_model_submenu(menu, ws, window, cx)
}

/// The "Regenerate with model" submenu — shared by `retry_items` and the
/// error row's "Retry turn" tail. Fewer than two switchable models hides
/// it — a plain retry covers that case.
pub(crate) fn retry_model_submenu(menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let picks = model_picks(ws, cx);
    if picks.len() < 2 {
        return menu;
    }
    let ws_sub = ws.clone();
    menu.submenu("Regenerate with model", window, cx, move |m, _w, _cx| {
        // Right-side checks keep the provider icon and the current-model
        // check visible together (left checks replace the icon).
        picks.iter().fold(m.check_side(Side::Right), |m, pick| m.item(model_item(&ws_sub, pick)))
    })
}

/// One submenu row: provider-prefixed label, checked on the live
/// selection; a pick selects the model then re-runs the turn.
fn model_item(ws: &Entity<Workspace>, pick: &ModelPick) -> PopupMenuItem {
    let ws = ws.clone();
    let pid = pick.provider.clone();
    let mid = pick.id.clone();
    let current = pick.current;
    PopupMenuItem::new(pick.label.clone()).icon(pick.icon).checked(current).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| {
            if !current {
                this.select_model(&pid, &mid, cx);
            }
            this.retry_last(cx);
        });
    })
}
