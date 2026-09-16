//! The add-provider wizard's step bodies: the driver card grid, the identity
//! form (label/id/accent), and the kind-specific config fields. Split from
//! `settings_provider_wizard` to stay under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::providers::ProviderKind;
use crate::views::settings::SettingsPanel;
use crate::views::settings_provider_detail::field;

/// Accent swatches offered in the Identity step — `None` renders as an
/// empty swatch meaning "no marker".
const ACCENTS: [Option<&str>; 7] = [
    None,
    Some("#3b82f6"),
    Some("#22c55e"),
    Some("#f97316"),
    Some("#ef4444"),
    Some("#a855f7"),
    Some("#06b6d4"),
];

/// Kinds shown in the driver grid that aren't implemented yet — they render
/// dimmed with a "Coming soon" badge and can't be selected.
const COMING_SOON: [(&str, IconName); 3] = [("Gemini", IconName::Sparkles), ("Copilot", IconName::Bot), ("Cursor", IconName::Box)];

/// Step 1: a two-column grid of provider-kind cards plus "Coming soon"
/// placeholders. Clicking a card selects the kind and re-seeds the
/// instance id + connection defaults for it. Kinds the detection scan
/// found installed carry a "Detected" chip.
pub(super) fn driver_step(panel: &Entity<SettingsPanel>, kind: ProviderKind, detected: &[ProviderKind], cx: &App) -> AnyElement {
    div()
        .id("wizard-driver")
        .test_support()
        .flex()
        .flex_wrap()
        .gap_2()
        .children(ProviderKind::ALL.into_iter().map(|k| kind_card(panel, k, k == kind, detected.contains(&k), cx)))
        .children(COMING_SOON.into_iter().map(|(name, icon)| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .w(px(250.))
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .opacity(0.5)
                .child(div().text_color(cx.theme().muted_foreground).child(icon))
                .child(div().text_xs().child(name))
                .child(div().flex_1())
                .child(div().text_xs().text_color(cx.theme().warning).child("Coming soon"))
        }))
        .into_any_element()
}

/// One selectable driver card — accent border + check when selected, a
/// "Detected" chip when the scan found this kind installed.
fn kind_card(panel: &Entity<SettingsPanel>, kind: ProviderKind, selected: bool, detected: bool, cx: &App) -> impl IntoElement {
    let panel = panel.clone();
    div()
        .id(SharedString::from(format!("wizard-kind-{}", kind.slug())))
        .test_support()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .w(px(250.))
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(if selected { cx.theme().accent } else { cx.theme().border })
        .child(div().text_color(cx.theme().muted_foreground).child(kind.info().icon))
        .child(div().text_xs().child(kind.info().label))
        .child(div().flex_1())
        .when(detected, |d| {
            d.child(
                div()
                    .id(SharedString::from(format!("wizard-detected-{}", kind.slug())))
                    .test_support()
                    .text_xs()
                    .px_1()
                    .rounded_sm()
                    .text_color(cx.theme().success)
                    .bg(cx.theme().success.opacity(0.12))
                    .child("Detected"),
            )
        })
        .when(selected, |d| d.child(div().text_color(cx.theme().accent).child(IconName::Check)))
        .on_click(move |_, window, cx| {
            panel.update(cx, |this, cx| this.reseed_wizard(kind, window, cx));
        })
}

/// Step 2: display label, routing id, and the accent swatch row.
pub(super) fn identity_step(panel: &Entity<SettingsPanel>, cx: &App) -> AnyElement {
    let (label, instance_id, accent) = match panel.read(cx).provider_wizard.as_ref() {
        Some(w) => (w.label.clone(), w.instance_id.clone(), w.accent.clone()),
        None => return div().into_any_element(),
    };
    div()
        .id("wizard-identity")
        .test_support()
        .flex()
        .flex_col()
        .gap_3()
        .child(field("Label", Input::new(&label).id("wizard-label").appearance(true).into_any_element()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Shown in the provider list. Optional."),
        )
        .child(field("Instance ID", Input::new(&instance_id).id("wizard-id").appearance(true).into_any_element()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Routing key used by threads and sessions. Letters, digits, '-', or '_'."),
        )
        .child(div().text_xs().font_semibold().child("Accent color"))
        .child(
            div()
                .flex()
                .gap_2()
                .children(ACCENTS.into_iter().map(|hex| swatch(panel, hex, accent.as_deref() == hex, cx))),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Optional marker shown in the picker."),
        )
        .into_any_element()
}

/// One accent swatch — a filled circle for a color, a hollow one for "none";
/// the selected swatch gets an accent ring.
fn swatch(panel: &Entity<SettingsPanel>, hex: Option<&'static str>, selected: bool, cx: &App) -> impl IntoElement {
    let panel = panel.clone();
    let fill = hex.and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok()).map(rgb);
    div()
        .id(SharedString::from(format!("wizard-accent-{}", hex.unwrap_or("none"))))
        .test_support()
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .size(px(20.))
        .rounded_full()
        .border_2()
        .border_color(if selected { cx.theme().accent } else { cx.theme().border })
        .when_some(fill, |d, c| d.bg(c))
        .when(hex.is_none(), |d| d.child(div().text_xs().text_color(cx.theme().muted_foreground).child("–")))
        .on_click(move |_, _, cx| {
            panel.update(cx, |this, cx| {
                if let Some(w) = this.provider_wizard.as_mut() {
                    w.accent = hex.map(str::to_string);
                }
                cx.notify();
            });
        })
}

/// Step 3: the kind's connection fields — acp/claude take a spawn command,
/// http takes url + key env, codex/sim have nothing to configure.
pub(super) fn config_step(panel: &Entity<SettingsPanel>, kind: ProviderKind, cx: &App) -> AnyElement {
    let (command, key_env) = match panel.read(cx).provider_wizard.as_ref() {
        Some(w) => (w.command.clone(), w.key_env.clone()),
        None => return div().into_any_element(),
    };
    let mut d = div().id("wizard-config").test_support().flex().flex_col().gap_3();
    match kind {
        ProviderKind::Acp | ProviderKind::ClaudeCli => {
            d = d.child(field("Command", Input::new(&command).id("wizard-command").appearance(true).into_any_element()));
        },
        ProviderKind::Http => {
            d = d
                .child(field("Endpoint URL", Input::new(&command).id("wizard-url").appearance(true).into_any_element()))
                .child(field("API key env var", Input::new(&key_env).id("wizard-key-env").appearance(true).into_any_element()));
        },
        ProviderKind::Ollama => {
            d = d.child(field("Base URL", Input::new(&command).id("wizard-url").appearance(true).into_any_element()));
        },
        ProviderKind::CodexCli | ProviderKind::Sim => {
            d = d.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Nothing to configure — this provider works out of the box."),
            );
        },
    }
    d.child(crate::views::settings_provider_test::wizard_test_row(panel, cx)).into_any_element()
}
