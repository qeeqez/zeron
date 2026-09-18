//! Click handlers, the inline rename editor and the queued-send chip for
//! `sidebar_row`'s chat rows: plain click selects, Cmd-click toggles the
//! bulk selection, double-click starts a rename, and the row's editor
//! commits on Enter or an outside click (Finder-style). Split from
//! `sidebar_row.rs` for the SLOC cap — the row body itself lives there.

use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Input, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Row click → select the chat (resolved by id — positions shift on
/// delete). A plain click also drops the multi-selection — including on
/// the active row, where `select_chat` early-returns. Focus lands on the
/// composer either way: the sidebar wrap is focusable now, so without the
/// explicit refocus a click on the already-active row would strand the
/// keyboard on the sidebar and typing would go nowhere.
pub(super) fn select_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.clear_chat_selection(cx);
        if let Some(ix) = this.chat_index(chat_id) {
            this.select_chat(ix, window, cx);
        }
        this.composer.update(cx, |s, cx| s.focus(window, cx));
    });
}

/// Cmd-click on a row → toggle the chat in the bulk-op selection and hand
/// the keyboard to the sidebar, so Enter renames the selected row (see
/// `Workspace::rename_selected_row`).
pub(super) fn toggle_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.toggle_chat_selection(chat_id, cx);
        let sidebar = this.sidebar_focus.clone();
        window.focus(&sidebar, cx);
    });
}

/// Double-click on the title → open the inline rename editor.
pub(super) fn rename_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        if let Some(ix) = this.chat_index(chat_id) {
            this.start_inline_rename(ix, window, cx);
        }
    });
}

/// Inline title editor — Enter commits, Escape cancels, and a mouse-down
/// anywhere outside the field commits (Finder-style).
pub(super) fn rename_editor(
    ws: Entity<Workspace>, input: Entity<InputState>, chat_id: u64,
) -> impl Fn(&mut Window, &mut App) -> AnyElement {
    let ws_out = ws.clone();
    let ws_enter = ws.clone();
    let ws_esc = ws.clone();
    move |_, _| {
        div()
            .flex_1()
            .min_w_0()
            .on_mouse_down_out({
                let ws = ws_out.clone();
                move |_, window, cx| {
                    ws.update(cx, |this, cx| this.commit_rename(window, cx));
                }
            })
            .on_action({
                let ws = ws_enter.clone();
                move |_: &InputEnter, window, cx| {
                    cx.stop_propagation();
                    ws.update(cx, |this, cx| this.commit_rename(window, cx));
                }
            })
            .on_action({
                let ws = ws_esc.clone();
                move |_: &InputEscape, window, cx| {
                    cx.stop_propagation();
                    ws.update(cx, |this, cx| this.cancel_inline_rename(window, cx));
                }
            })
            .child(Input::new(&input).id(("rename-input", chat_id)).xsmall().w_full())
            .into_any_element()
    }
}

/// The "+N" queued-send chip: same pill geometry as the titlebar's unread
/// badge, muted instead of red. Clicking selects the chat — its composer
/// holds the queue UI — and stops the row's own click (rename on
/// double-click) from seeing the press.
pub(super) fn queue_badge(chat_id: u64, queued: usize, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let tip = format!("{queued} queued");
    div()
        .id(("queue-badge", chat_id))
        .test_support()
        .aria_label(tip.clone())
        .min_w(px(14.))
        .h(px(14.))
        .px(px(3.))
        .rounded_full()
        .bg(cx.theme().muted)
        .text_color(cx.theme().muted_foreground)
        .text_size(px(9.))
        .flex()
        .items_center()
        .justify_center()
        .child(format!("+{queued}"))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .on_click({
            let ws = ws.clone();
            move |_, window, cx| {
                cx.stop_propagation();
                select_row(&ws, chat_id, window, cx);
            }
        })
}
