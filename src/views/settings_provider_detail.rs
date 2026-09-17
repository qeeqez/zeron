//! The Providers detail panel (right side of the master-detail split): the
//! selected instance's header, display-name input, kind-specific connection
//! fields, and the Models block with per-model enable + ordering.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::auth::{self, AuthState};
use crate::providers::{ProviderInstance, ProviderKind};
use crate::views::settings_provider_env::variables_section;
use crate::views::settings_providers::{ProviderInputs, code_row};
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
        .child(crate::views::settings_provider_health::health_line(p, s, cx))
        .when_some(inputs.clone(), |d, inputs| {
            d.child(s.search.wrap(
                "Display name",
                field(
                    "Display name",
                    Input::new(&inputs.name)
                        .id(SharedString::from(format!("provider-name-{}", p.id)))
                        .appearance(true)
                        .into_any_element(),
                ),
            ))
            .children(connection_fields(p, &inputs, &s.search))
        })
        .child(account_section(p, inputs.as_ref(), s, cx))
        // The simulator spawns nothing — Variables would be a no-op.
        .when(!matches!(p.kind, ProviderKind::Sim), |d| {
            d.child(variables_section(p, s.provider_env_inputs.get(&p.id).map_or(&[][..], Vec::as_slice), s, cx))
        })
        .child(models_section(p, s, cx))
}

/// The kind-specific connection inputs: acp/claude take a spawn command,
/// http takes an endpoint URL plus the env var holding its token.
fn connection_fields(p: &ProviderInstance, inputs: &ProviderInputs, search: &crate::views::settings_search::SearchCtx) -> Vec<AnyElement> {
    let conn_field = |label: &'static str, id: &str, input: &Entity<InputState>| {
        search.wrap(
            label,
            field(
                label,
                Input::new(input)
                    .id(SharedString::from(format!("{id}-{}", p.id)))
                    .appearance(true)
                    .into_any_element(),
            ),
        )
    };
    match p.kind {
        ProviderKind::Acp | ProviderKind::ClaudeCli | ProviderKind::Mcp => {
            vec![conn_field("Command", "provider-command", &inputs.command)]
        },
        ProviderKind::Http => vec![
            conn_field("Endpoint URL", "provider-url", &inputs.command),
            conn_field("API key env var", "provider-key-env", &inputs.key_env),
        ],
        ProviderKind::Ollama => vec![conn_field("Base URL", "provider-url", &inputs.command)],
        ProviderKind::CodexCli | ProviderKind::Sim => Vec::new(),
    }
}

/// The Account block: auth status line, sign-in/out controls, the device
/// URL/code while a flow runs, and the paste-back input for flows that
/// need a code. Hidden for kinds with no credentials at all (sim).
fn account_section(p: &ProviderInstance, inputs: Option<&ProviderInputs>, s: &SettingsView, cx: &App) -> AnyElement {
    let state = s.ws.read(cx).auth_state(&p.id);
    if matches!(state, AuthState::NotRequired) && !auth::can_sign_in(p.kind) {
        return div().into_any_element();
    }
    let (id_in, id_out, id_cancel, id_code) = (p.id.clone(), p.id.clone(), p.id.clone(), p.id.clone());
    let (ws_in, ws_out, ws_cancel, ws_code) = (s.ws.clone(), s.ws.clone(), s.ws.clone(), s.ws.clone());
    let status_color = match &state {
        AuthState::SignedIn(_) => cx.theme().success,
        AuthState::SignedOut => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
    let mut section = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(group_label("Account", &s.search, cx))
        .child(div().text_xs().text_color(status_color).child(state.detail_status()));
    if let Some(err) = s.ws.read(cx).auth_error(&p.id) {
        section = section.child(div().text_xs().text_color(cx.theme().danger).child(err.to_string()));
    }
    match &state {
        AuthState::SigningIn(prompt) | AuthState::AwaitingCode(prompt) => {
            if !prompt.is_empty() {
                section = section.child(div().text_xs().text_color(cx.theme().muted_foreground).child(prompt.clone()));
            }
            if let (AuthState::AwaitingCode(_), Some(inputs)) = (&state, inputs) {
                section = section.child(code_row(&id_code, inputs, &ws_code));
            }
            section = section.child(
                Button::new(SharedString::from(format!("auth-cancel-{id_cancel}")))
                    .label("Cancel")
                    .small()
                    .ghost()
                    .on_click(move |_, _, cx| ws_cancel.update(cx, |this, cx| this.cancel_sign_in(&id_cancel, cx))),
            );
        },
        AuthState::SignedIn(_) if auth::can_sign_in(p.kind) => {
            section = section.child(
                Button::new(SharedString::from(format!("auth-sign-out-{id_out}")))
                    .label("Sign out")
                    .icon(IconName::LogOut)
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| ws_out.update(cx, |this, cx| this.sign_out(&id_out, cx))),
            );
        },
        AuthState::SignedOut | AuthState::Unknown if auth::can_sign_in(p.kind) => {
            section = section.child(
                Button::new(SharedString::from(format!("auth-sign-in-{id_in}")))
                    .label("Sign in")
                    .icon(IconName::LogIn)
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| ws_in.update(cx, |this, cx| this.sign_in(&id_in, cx))),
            );
        },
        // Env-keyed kinds: no flow — hint at the var instead.
        AuthState::SignedOut => {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Set {} in your environment.", p.key_env)),
            );
        },
        _ => {},
    }
    section.into_any_element()
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
        .child(group_label("Models", &s.search, cx))
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
pub(crate) fn icon_btn(
    id: &str, icon: IconName, on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .test_support()
        .cursor_pointer()
        .child(icon)
        .on_click(on_click)
}
