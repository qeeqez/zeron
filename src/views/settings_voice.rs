//! The Voice settings section: the dictation switch, language + on-device
//! pickers, and a test-mic button that runs a real take and shows what the
//! recognizer heard.

use crate::views::settings::SettingsPanel;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::select::Select;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings_sections::{SettingsView, group_label, toggle_row};
use crate::voice::DictationPhase;
use crate::workspace::Workspace;

/// (label, BCP-47 locale id) rows for the language select; empty id = the
/// recognizer's default locale.
pub(crate) const VOICE_LANGUAGES: &[(&str, &str)] = &[
    ("System default", ""),
    ("English (US)", "en-US"),
    ("English (UK)", "en-GB"),
    ("German", "de-DE"),
    ("French", "fr-FR"),
    ("Spanish", "es-ES"),
    ("Italian", "it-IT"),
    ("Portuguese (BR)", "pt-BR"),
    ("Japanese", "ja-JP"),
    ("Korean", "ko-KR"),
    ("Chinese (Simplified)", "zh-CN"),
    ("Chinese (Traditional)", "zh-TW"),
];

/// The dictation-language `SelectState`: items are `VOICE_LANGUAGES`
/// labels; Confirm maps back to the locale id (empty = system default) and
/// persists via `set_voice_language`.
pub(crate) fn language_picker(
    ws: &WeakEntity<Workspace>, settings: &crate::persist::Settings, window: &mut Window, cx: &mut Context<SettingsPanel>,
) -> Entity<gpui_kit::component::select::SelectState<Vec<String>>> {
    use gpui_kit::component::select::{SelectEvent, SelectState};
    let select = cx.new(|cx| {
        let selected = VOICE_LANGUAGES
            .iter()
            .position(|(_, id)| *id == settings.voice_language)
            .map(gpui_kit::component::IndexPath::new);
        SelectState::new(VOICE_LANGUAGES.iter().map(|(label, _)| label.to_string()).collect::<Vec<_>>(), selected, window, cx)
    });
    let ws = ws.clone();
    cx.subscribe_in(&select, window, move |_, _, event: &SelectEvent<Vec<String>>, _window, cx| {
        let SelectEvent::Confirm(label) = event;
        let id = VOICE_LANGUAGES
            .iter()
            .find(|(l, _)| Some(*l) == label.as_deref())
            .map(|(_, id)| *id)
            .unwrap_or_default();
        let _ = ws.update(cx, |this, cx| this.set_voice_language(id.to_string(), cx));
    })
    .detach();
    select
}

/// The Voice content pane.
pub(crate) fn voice_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let ws = s.ws.clone();
    let busy = s.voice_phase != DictationPhase::Idle;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Dictation", cx))
        .child(toggle_row(("toggle-voice", "Enable dictation"), s.voice_enabled, ws.clone(), Workspace::set_voice_enabled))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Dictate into the composer with the mic button or Cmd-Shift-D."),
        )
        .child(voice_row(
            "Language",
            "Recognition locale — System default follows macOS.",
            div()
                .w(px(220.))
                .child(Select::new(&s.voice_language_select).id("voice-language").small().appearance(true)),
            cx,
        ))
        .child(toggle_row(("toggle-voice-device", "On-device recognition"), s.voice_on_device, ws.clone(), Workspace::set_voice_on_device))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("On-device keeps audio on this Mac but supports fewer languages."),
        )
        .child(group_label("Microphone", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("macOS asks for Microphone and Speech Recognition access on first use."),
        )
        .child(
            div().flex().items_center().gap_2().child(
                div().id("voice-test-mic").test_support().child(
                    Button::new("voice-test-mic-btn")
                        .ghost()
                        .small()
                        .icon(IconName::Mic)
                        .label(if busy { "Stop & transcribe" } else { "Test microphone" })
                        .on_click({
                            let ws = ws.clone();
                            move |_, window, cx| {
                                ws.update(cx, |this, cx| this.toggle_dictation_test(window, cx));
                            }
                        }),
                ),
            ),
        )
        .children(test_status(s, cx))
}

/// What the test row reports: live phase while a take runs, then the last
/// result.
fn test_status(s: &SettingsView, cx: &App) -> Option<AnyElement> {
    let text = match s.voice_phase {
        DictationPhase::Recording => s.voice_test_result.clone().unwrap_or_else(|| "Listening…".into()),
        DictationPhase::Transcribing => "Transcribing…".into(),
        DictationPhase::Idle => s.voice_test_result.clone()?,
    };
    Some(
        div()
            .id("voice-test-result")
            .test_support()
            .aria_label(text.clone())
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .into_any_element(),
    )
}

/// Label + caption left, control right — same shape as the General
/// section's `default_row`, local so the caption id stays unique.
fn voice_row(label: &'static str, caption: &'static str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div().flex_1().min_w_0().flex().flex_col().child(div().text_sm().child(label)).child(
                div()
                    .id(SharedString::from(format!("voice-caption-{}", label.to_lowercase())))
                    .test_support()
                    .aria_label(caption)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(caption),
            ),
        )
        .child(control)
}
