//! The bulk-op bar pinned above the sidebar footer while chats are
//! Cmd-click-selected: Archive/Delete apply to the whole set, Clear drops it.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// "Archive (N) · Delete (N) · Clear" — `n` is the live selected count.
/// Buttons sit in observed wrappers so tests can click them by id.
pub(super) fn selection_bar(n: usize, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("chat-selection-bar")
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .border_t_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            div().id("archive-selected").test_support().child(
                Button::new("archive-selected-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Archive)
                    .label(format!("Archive ({n})"))
                    .on_click(cx.listener(|this, _, window, cx| this.archive_selected(window, cx))),
            ),
        )
        .child(
            div().id("delete-selected").test_support().child(
                Button::new("delete-selected-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Delete)
                    .label(format!("Delete ({n})"))
                    .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))),
            ),
        )
        .child(div().flex_1())
        .child(
            div().id("clear-selected").test_support().child(
                Button::new("clear-selected-btn")
                    .ghost()
                    .xsmall()
                    .label("Clear")
                    .on_click(cx.listener(|this, _, _, cx| this.clear_chat_selection(cx))),
            ),
        )
}
