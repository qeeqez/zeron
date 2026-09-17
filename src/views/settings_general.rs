//! The General settings section: thread defaults (model, permissions,
//! workspace) plus the notification/access/word-wrap controls that predate
//! the nav split. The default-model control lives in
//! `settings_default_model.rs` — the composer's `model_picker` can't be
//! reused because its rows call `select_model`, which switches the ACTIVE
//! thread's backend; the default must leave the live selection alone.

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::AccessMode;
use crate::open_in::PreferredEditor;
use crate::views::settings::SettingsPanel;
use crate::views::settings_default_model::default_model_picker;
use crate::views::settings_sections::{SettingsView, group_label, toggle_row};
use crate::workspace::Workspace;
use crate::worktree::WorkspaceMode;

/// The General content pane: thread defaults first, then the controls that
/// apply to the current window/thread.
pub(crate) fn general_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Thread defaults", &s.search, cx))
        .child(default_row(
            "Model",
            "Default model for new threads. Projects can override it.",
            default_model_picker(&s.ws, cx).into_any_element(),
            &s.search,
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
            &s.search,
            cx,
        ))
        .child(default_row(
            "Workspace",
            "Where new threads start.",
            div()
                .w(px(220.))
                .child(Select::new(&s.workspace_select).id("default-workspace").small().appearance(true)),
            &s.search,
            cx,
        ))
        .child(group_label("Files", &s.search, cx))
        .child(default_row(
            "Editor",
            "Editor used by \"Open in Editor\" on file rows.",
            div()
                .w(px(220.))
                .child(Select::new(&s.editor_select).id("preferred-editor").small().appearance(true)),
            &s.search,
            cx,
        ))
        .child(group_label("Notifications", &s.search, cx))
        .child(toggle_row(
            ("toggle-notify", "Notify on reply complete"),
            s.notify,
            ws.clone(),
            |this, next, _w, _cx| {
                this.notify_on_done = next;
            },
            &s.search,
        ))
        .child(toggle_row(
            ("toggle-notify-sound", "Notification sound"),
            s.notify_sound,
            ws.clone(),
            |this, next, _w, _cx| {
                this.notify_sound = next;
            },
            &s.search,
        ))
        .child(toggle_row(
            ("toggle-notify-background", "Notify on background replies"),
            s.notify_background,
            ws.clone(),
            |this, next, _w, _cx| {
                this.notify_background = next;
            },
            &s.search,
        ))
        .child(group_label("Agent access", &s.search, cx))
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
        .child(group_label("Messages", &s.search, cx))
        .child(toggle_row(
            ("toggle-wrap", "Word wrap"),
            s.word_wrap,
            ws.clone(),
            |this, next, _w, cx| {
                this.word_wrap = next;
                this.scroller.update(cx, |s, cx| s.remeasure(cx));
            },
            &s.search,
        ))
        .child(toggle_row(
            ("toggle-timestamps", "Show message timestamps"),
            s.show_timestamps,
            ws.clone(),
            |this, next, _w, cx| {
                this.show_timestamps = next;
                this.scroller.update(cx, |s, cx| s.remeasure(cx));
            },
            &s.search,
        ))
        .child(group_label("Usage", &s.search, cx))
        .child(default_row(
            "Budget alert",
            "Warn when a chat's spend passes this cap — per-chat overrides live in the chat's ⋯ menu. Enter applies it.",
            div()
                .w(px(220.))
                .child(Input::new(&s.ws.read(cx).budget_cap_input).id("budget-cap-input").small().appearance(true)),
            &s.search,
            cx,
        ))
        .child(group_label("System", &s.search, cx))
        .child(toggle_row(
            ("toggle-global-hotkey", "Global hotkey"),
            s.ws.read(cx).global_hotkey_enabled,
            ws.clone(),
            |this, next, _w, cx| {
                this.set_global_hotkey_enabled(next, cx);
            },
            &s.search,
        ))
        .child(default_row(
            "Summon shortcut",
            "System-wide chord that focuses the app — e.g. cmd-shift-space. Enter applies it.",
            div()
                .w(px(220.))
                .child(Input::new(&s.ws.read(cx).hotkey_input).id("global-hotkey-input").small().appearance(true)),
            &s.search,
            cx,
        ))
        .children(s.ws.read(cx).hotkey_error.iter().map(|e| {
            div()
                .id("global-hotkey-error")
                .test_support()
                .aria_label(e.clone())
                .text_xs()
                .text_color(cx.theme().danger)
                .child(e.clone())
        }))
}

/// One settings row: label + muted caption on the left, the control pinned
/// right. The caption div carries an `aria_label` so tests (and assistive
/// tech) can read it — snapshots don't capture rendered text.
fn default_row(
    label: &'static str, caption: &'static str, control: impl IntoElement, search: &crate::views::settings_search::SearchCtx, cx: &App,
) -> AnyElement {
    search.wrap(
        label,
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
            .child(control),
    )
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

/// The preferred-editor select: `PreferredEditor::ALL` labels with the
/// persisted pick selected; Confirm writes `Workspace::preferred_editor`
/// via `set_preferred_editor`. Lives here (not inline in `SettingsPanel::new`)
/// to keep `settings.rs` under the SLOC cap.
pub(crate) fn editor_picker(
    ws: &WeakEntity<Workspace>, settings: &crate::persist::Settings, window: &mut Window, cx: &mut Context<SettingsPanel>,
) -> Entity<SelectState<Vec<String>>> {
    let editor = PreferredEditor::from_name(&settings.preferred_editor);
    let selected = PreferredEditor::ALL.iter().position(|e| *e == editor).map(gpui_kit::component::IndexPath::new);
    let select = cx.new(|cx| SelectState::new(PreferredEditor::ALL.map(|e| e.label().to_string()).to_vec(), selected, window, cx));
    let ws = ws.clone();
    cx.subscribe_in(&select, window, move |_, _, event: &SelectEvent<Vec<String>>, _window, cx| {
        let SelectEvent::Confirm(label) = event;
        if let Some(label) = label {
            let _ = ws.update(cx, |this, cx| this.set_preferred_editor(PreferredEditor::from_label(label), cx));
        }
    })
    .detach();
    select
}
