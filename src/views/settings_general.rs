//! The General settings section: thread defaults (model, permissions,
//! workspace) plus the notification/access/word-wrap controls that predate
//! the nav split. The default-model control lives in
//! `settings_default_model.rs` — the composer's `model_picker` can't be
//! reused because its rows call `select_model`, which switches the ACTIVE
//! thread's backend; the default must leave the live selection alone.

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::select::Select;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::AccessMode;
use crate::views::settings_default_model::default_model_picker;
use crate::views::settings_sections::{SettingsView, group_label, toggle_row};
use crate::worktree::WorkspaceMode;

/// The General content pane: thread defaults first, then the controls that
/// apply to the current window/thread.
pub(crate) fn general_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Thread defaults", cx))
        .child(default_row(
            "Model",
            "Default model for new threads. Projects can override it.",
            default_model_picker(&s.ws, cx).into_any_element(),
            cx,
        ))
        .child(default_row(
            "Permissions",
            "Default permissions for new threads.",
            div().w(px(220.)).child(
                Select::new(&s.permissions_select)
                    .id("default-permissions")
                    .small()
                    .appearance(true)
                    .cleanable(true)
                    .placeholder("Follow current"),
            ),
            cx,
        ))
        .child(default_row(
            "Workspace",
            "Where new threads start.",
            div()
                .w(px(220.))
                .child(Select::new(&s.workspace_select).id("default-workspace").small().appearance(true)),
            cx,
        ))
        .child(group_label("Notifications", cx))
        .child(toggle_row(("toggle-notify", "Notify on reply complete"), s.notify, ws.clone(), |this, next, _w, _cx| {
            this.notify_on_done = next;
        }))
        .child(toggle_row(("toggle-notify-sound", "Notification sound"), s.notify_sound, ws.clone(), |this, next, _w, _cx| {
            this.notify_sound = next;
        }))
        .child(group_label("Agent access", cx))
        .child(div().flex().items_center().gap_2().text_xs().children(AccessMode::ALL.into_iter().map(|mode| {
            let btn = Button::new(SharedString::from(mode.name())).label(mode.label()).on_click({
                let ws = ws.clone();
                move |_, _, cx| {
                    ws.update(cx, |this, cx| this.set_access(mode, cx));
                }
            });
            if mode == s.access { btn.primary() } else { btn.outline() }
        })))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Filesystem access for Agent-mode turns — Plan and Ask always stay read-only"),
        )
        .child(group_label("Messages", cx))
        .child(toggle_row(("toggle-wrap", "Word wrap"), s.word_wrap, ws.clone(), |this, next, _w, cx| {
            this.word_wrap = next;
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
        }))
}

/// One settings row: label + muted caption on the left, the control pinned
/// right. The caption div carries an `aria_label` so tests (and assistive
/// tech) can read it — snapshots don't capture rendered text.
fn default_row(label: &'static str, caption: &'static str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div().flex_1().min_w_0().flex().flex_col().child(div().text_sm().child(label)).child(
                div()
                    .id(SharedString::from(format!("caption-{}", label.to_lowercase())))
                    .test_support()
                    .aria_label(caption)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(caption),
            ),
        )
        .child(control)
}

/// Display label for a `WorkspaceMode` in the default-workspace select.
pub(crate) fn workspace_mode_label(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Checkout => "Project checkout",
        WorkspaceMode::Worktree => "Git worktree",
    }
}

/// Inverse of `workspace_mode_label` for the select's Confirm event.
pub(crate) fn workspace_mode_from_label(label: &str) -> WorkspaceMode {
    WorkspaceMode::ALL.iter().copied().find(|m| workspace_mode_label(*m) == label).unwrap_or_default()
}

/// Inverse of `AccessMode::label` for the permissions select's Confirm
/// event — `from_name` parses persisted names, not display labels.
pub(crate) fn access_mode_from_label(label: &str) -> AccessMode {
    AccessMode::ALL.iter().copied().find(|m| m.label() == label).unwrap_or_default()
}
