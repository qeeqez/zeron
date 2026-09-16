//! The Project settings section: the per-project setup script — a shell
//! command run (`sh -c`) inside each new thread worktree, the desktop
//! counterpart of Codex cloud's environment setup. Persisted to the
//! project's `state.json` (`ProjectState.setup_script`); see
//! `crate::setup_script` for the runner.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings_sections::{SettingsView, group_label};

/// The Project content pane: the setup-script field with its dirty/saved
/// state, then a note on when it runs.
pub(crate) fn project_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let (saved, draft) = {
        let ws = s.ws.read(cx);
        (ws.setup_script.clone(), ws.setup_script_input.read(cx).value().to_string())
    };
    let dirty = draft.trim() != saved.trim();
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Setup script", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Runs inside each new thread worktree (sh -c) — install dependencies, symlink env files. Failures land as a note; they never block the thread."),
        )
        .child(
            div()
                .id("setup-script-field")
                .test_support()
                .child(Textarea::new(&s.setup_script_input).aria_label("Setup script")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("setup-script-save")
                        .label("Save")
                        .small()
                        .primary()
                        .disabled(!dirty)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.save_setup_script(cx));
                        }),
                )
                .child(
                    div()
                        .id("setup-script-status")
                        .test_support()
                        .aria_label(if dirty { "Unsaved changes" } else { "Saved" })
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if dirty { "Unsaved changes" } else { "Saved" }),
                ),
        )
}
