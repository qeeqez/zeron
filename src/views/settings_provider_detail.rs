//! The Providers detail panel (right side of the master-detail split): the
//! selected instance's header, display-name input, kind-specific connection
//! fields, and the Models block with per-model enable + ordering.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::providers::{ProviderInstance, ProviderKind};
use crate::views::settings_providers::ProviderInputs;
use crate::views::settings_sections::{SettingsView, group_label};

/// The right pane for the selected instance: header (icon, name, id, remove),
/// display-name input, kind-specific connection fields, then the model list.
pub(crate) fn detail_panel(p: Option<&ProviderInstance>, s: &SettingsView, cx: &App) -> impl IntoElement {
    let Some(p) = p else {
        return div()
            .id("provider-detail")
            .test_support()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Select a provider to configure it."));
    };
    let inputs = s.provider_inputs.get(&p.id).cloned();
    let active = s.ws.read(cx).selected_provider() == Some(p.id.as_str());
    let (id_rm, ws_rm) = (p.id.clone(), s.ws.clone());
    div()
        .id("provider-detail")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().child(p.kind.info().icon))
                .child(div().text_sm().font_semibold().child(p.name.clone()))
                .when(active, |d| d.child(div().text_xs().text_color(cx.theme().accent).child("· active")))
                .child(div().flex_1())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(p.id.clone()))
                .child(icon_btn(&format!("provider-remove-{}", p.id), IconName::Trash, move |_, _, cx| {
                    ws_rm.update(cx, |this, cx| this.remove_provider(&id_rm, cx));
                })),
        )
        .when_some(inputs, |d, inputs| {
            d.child(field(
                "Display name",
                Input::new(&inputs.name)
                    .id(SharedString::from(format!("provider-name-{}", p.id)))
                    .appearance(true)
                    .into_any_element(),
            ))
            .children(connection_fields(p, &inputs))
        })
        .child(models_section(p, s, cx))
}

/// The kind-specific connection inputs: acp/claude take a spawn command,
/// http takes an endpoint URL plus the env var holding its token.
fn connection_fields(p: &ProviderInstance, inputs: &ProviderInputs) -> Vec<AnyElement> {
    match p.kind {
        ProviderKind::Acp | ProviderKind::ClaudeCli => vec![field(
            "Command",
            Input::new(&inputs.command)
                .id(SharedString::from(format!("provider-command-{}", p.id)))
                .appearance(true)
                .into_any_element(),
        )],
        ProviderKind::Http => vec![
            field(
                "Endpoint URL",
                Input::new(&inputs.command)
                    .id(SharedString::from(format!("provider-url-{}", p.id)))
                    .appearance(true)
                    .into_any_element(),
            ),
            field(
                "API key env var",
                Input::new(&inputs.key_env)
                    .id(SharedString::from(format!("provider-key-env-{}", p.id)))
                    .appearance(true)
                    .into_any_element(),
            ),
        ],
        ProviderKind::CodexCli | ProviderKind::Sim => Vec::new(),
    }
}

/// A labeled input row for the detail panel and the wizard.
pub(crate) fn field(label: &'static str, input: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .text_xs()
        .child(div().font_semibold().child(label))
        .child(input)
        .into_any_element()
}

/// The Models block: enabled models (with up/down ordering) under "All",
/// disabled ones under "Hidden from picker". Counts sit on the header line.
fn models_section(p: &ProviderInstance, s: &SettingsView, cx: &App) -> impl IntoElement {
    let models = s.ws.read(cx).models_config_for(&p.id);
    let hidden = models.iter().filter(|(_, on)| !on).count();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(group_label("Models", cx))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
            "{} models · {} hidden — order and visibility apply to the picker.",
            models.len(),
            hidden
        )))
        .children(models.iter().filter(|(_, on)| *on).map(|(m, _)| model_row(&p.id, m, true, s, cx)))
        .when(hidden > 0, |d| {
            d.child(div().pt_1().text_xs().text_color(cx.theme().muted_foreground).child("Hidden from picker"))
                .children(models.iter().filter(|(_, on)| !on).map(|(m, _)| model_row(&p.id, m, false, s, cx)))
        })
}

/// One model row: enable `Switch`, label + id, and up/down reorder controls
/// (hidden models keep their order but don't need the arrows).
fn model_row(pid: &str, m: &crate::model::ModelInfo, on: bool, s: &SettingsView, cx: &App) -> impl IntoElement {
    let pid = pid.to_string();
    let mid = m.id.to_string();
    let (ws_t, ws_up, ws_dn) = (s.ws.clone(), s.ws.clone(), s.ws.clone());
    let (id_t, id_up, id_dn) = (mid.clone(), mid.clone(), mid.clone());
    let (p_t, p_up, p_dn) = (pid.clone(), pid.clone(), pid.clone());
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(
            Switch::new(SharedString::from(format!("model-toggle-{pid}-{mid}")))
                .checked(on)
                .small()
                .accessibility_label(format!("Enable {}", m.label))
                .on_click(move |next, _, cx| {
                    ws_t.update(cx, |this, cx| this.set_model_enabled(&p_t, &id_t, *next, cx));
                }),
        )
        .child(div().child(m.label.clone()))
        .child(div().text_color(cx.theme().muted_foreground).child(mid.clone()))
        .child(div().flex_1())
        .when(on, |d| {
            d.child(icon_btn(&format!("model-up-{pid}-{id_up}"), IconName::ChevronUp, move |_, _, cx| {
                ws_up.update(cx, |this, cx| this.move_model(&p_up, &id_up, -1, cx));
            }))
            .child(icon_btn(&format!("model-down-{pid}-{id_dn}"), IconName::ChevronDown, move |_, _, cx| {
                ws_dn.update(cx, |this, cx| this.move_model(&p_dn, &id_dn, 1, cx));
            }))
        })
}

/// A small clickable icon — the shared shape for remove/reorder controls.
fn icon_btn(id: &str, icon: IconName, on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .test_support()
        .cursor_pointer()
        .child(icon)
        .on_click(on_click)
}
