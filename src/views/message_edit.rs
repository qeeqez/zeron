//! Inline editor for a user message being resent — replaces the bubble
//! while `Workspace::editing` points at the row. Enter resends (forking
//! the turn), Escape or Cancel abandons the edit; the action handlers
//! mirror the sidebar's rename editor.

use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Textarea, TextareaState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

pub(super) fn message_editor(ix: usize, input: &Entity<TextareaState>, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let ws_enter = ws.clone();
    let ws_esc = ws.clone();
    let ws_cancel = ws.clone();
    let ws_resend = ws.clone();
    div()
        .id(("msg-edit", ix))
        .test_support()
        .flex()
        .flex_col()
        .gap_1()
        .px_4()
        .py_2()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().accent)
        .bg(cx.theme().background)
        .on_action(move |_: &InputEnter, window, cx| {
            cx.stop_propagation();
            ws_enter.update(cx, |this, cx| this.commit_edit(window, cx));
        })
        .on_action(move |_: &InputEscape, window, cx| {
            cx.stop_propagation();
            ws_esc.update(cx, |this, cx| this.cancel_edit(window, cx));
        })
        .child(Textarea::new(input).appearance(false).bordered(false))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Enter to resend · Esc to cancel")
                .child(div().flex_1())
                .child(
                    div()
                        .id(("msg-edit-cancel", ix))
                        .test_support()
                        .cursor_pointer()
                        .child("Cancel")
                        .on_click(move |_, window, cx| {
                            ws_cancel.update(cx, |this, cx| this.cancel_edit(window, cx));
                        }),
                )
                .child(
                    div()
                        .id(("msg-edit-resend", ix))
                        .test_support()
                        .cursor_pointer()
                        .text_color(cx.theme().accent)
                        .child("Resend")
                        .on_click(move |_, window, cx| {
                            ws_resend.update(cx, |this, cx| this.commit_edit(window, cx));
                        }),
                ),
        )
        .into_any_element()
}
