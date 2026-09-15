//! The composer's dictation chrome: the mic button (idle / recording /
//! transcribing) and the status line under the input. Split from
//! `composer.rs` to stay under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::voice::DictationPhase;
use crate::workspace::Workspace;

/// Mic toggles dictation; red while recording, spinner while transcribing.
/// With voice off it opens the Voice settings section instead of recording.
pub(crate) fn dictate_button(phase: DictationPhase, cx: &mut Context<Workspace>) -> impl IntoElement {
    div().id("dictate").test_support().child(
        Button::new("dictate-btn")
            .ghost()
            .icon(match phase {
                DictationPhase::Recording => IconName::MicOff,
                DictationPhase::Transcribing => IconName::Loader,
                DictationPhase::Idle => IconName::Mic,
            })
            .when(phase == DictationPhase::Recording, |b| b.text_color(cx.theme().danger))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_dictation(window, cx))),
    )
}

/// Dictation status line: live partial transcript while listening, or the
/// failure reason after a take ends.
pub(crate) fn dictate_note(note: String, is_error: bool, cx: &App) -> impl IntoElement {
    div()
        .id("dictate-note")
        .test_support()
        .aria_label(note.clone())
        .pt_1()
        .text_xs()
        .text_color(if is_error { cx.theme().danger } else { cx.theme().muted_foreground })
        .child(note)
}
