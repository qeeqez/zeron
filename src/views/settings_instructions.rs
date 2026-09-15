//! The Custom Instructions settings section: a multiline field for the
//! global instructions (persisted to `Settings.instructions`, applied to
//! every turn) plus a note about the project's own instructions file —
//! `AGENTS.md`, `CLAUDE.md`, or `.rixl/instructions.md` at the project
//! root is merged in after the global text (see `crate::instructions`).
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings_sections::{SettingsView, group_label};

/// The Custom Instructions content pane: the global field with its
/// dirty/saved state, then the project-file note.
pub(crate) fn instructions_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let (saved, draft, root) = {
        let ws = s.ws.read(cx);
        (ws.instructions.clone(), ws.instructions_input.read(cx).value().to_string(), ws.project.root().to_path_buf())
    };
    let dirty = draft.trim() != saved.trim();
    let (note_label, note_text) = project_note(&root);
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Global instructions", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Applied to every turn, ahead of the project's own instructions file."),
        )
        .child(
            div()
                .id("instructions-field")
                .test_support()
                .child(Textarea::new(&s.instructions_input).aria_label("Custom instructions")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("instructions-save")
                        .label("Save")
                        .small()
                        .primary()
                        .disabled(!dirty)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.save_instructions(cx));
                        }),
                )
                .child(
                    div()
                        .id("instructions-status")
                        .test_support()
                        .aria_label(if dirty { "Unsaved changes" } else { "Saved" })
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if dirty { "Unsaved changes" } else { "Saved" }),
                ),
        )
        .child(group_label("Project instructions", cx))
        .child(
            div()
                .id("instructions-project-note")
                .test_support()
                .aria_label(note_label)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(note_text),
        )
}

/// (aria label, rendered text) for the project-file note — which file was
/// found under `root`, or the candidate list when none exists.
fn project_note(root: &std::path::Path) -> (String, String) {
    match crate::instructions::project_file(root) {
        Some((name, text)) => {
            let label = format!("{name} is appended after the global instructions");
            (label.clone(), format!("{label} — {} chars.", text.chars().count()))
        },
        None => {
            let files = crate::instructions::PROJECT_FILES.join(", ");
            let label = format!("No project instructions file — add one of: {files}");
            (label.clone(), format!("{label} at the project root."))
        },
    }
}
