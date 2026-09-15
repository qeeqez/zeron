//! Providers settings section: enable/disable each model provider and
//! configure the HTTP transport. Provider rows read the workspace directly
//! (enabled set, active provider, catalog size) so toggles re-render in
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::PROVIDERS;
use crate::views::settings_sections::{SettingsView, group_label};
use crate::workspace::Workspace;

/// The Providers content pane: one toggle row per provider plus the HTTP
/// endpoint inputs (the http provider's only configuration).
pub fn providers_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Model providers", cx))
        .children(PROVIDERS.iter().map(|p| provider_row(p, &s.ws, cx)))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Disabled providers are hidden from the model picker. The last enabled provider can't be turned off."),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("Active backend: {}", s.backend)),
        )
        .child(group_label("HTTP endpoint", cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .child(div().text_color(cx.theme().muted_foreground).child("URL"))
                .child(Input::new(&s.url_input).appearance(true))
                .child(div().text_color(cx.theme().muted_foreground).child("API key env var"))
                .child(Input::new(&s.key_input).appearance(true)),
        )
}

/// One provider row: label + tagline + catalog size, an "active" marker
/// when it's the selected provider, and a check/X toggle at the right.
fn provider_row(p: &crate::model::ProviderInfo, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let (enabled, active, count) = ws.read_with(cx, |w, _| {
        (!w.disabled_providers.iter().any(|d| d == p.id), w.provider == p.id, w.model_catalog.get(p.id).map_or(0, Vec::len))
    });
    let ws2 = ws.clone();
    let id = p.id;
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(div().font_semibold().child(p.label))
        .child(div().text_color(cx.theme().muted_foreground).child(p.tagline))
        .child(div().text_color(cx.theme().muted_foreground).child(if count == 1 {
            "· 1 model".to_string()
        } else {
            format!("· {count} models")
        }))
        .when(active, |d| d.child(div().text_color(cx.theme().accent).child("· active")))
        .child(div().flex_1())
        .child(
            div()
                .id(SharedString::from(format!("provider-toggle-{id}")))
                .test_support()
                .cursor_pointer()
                .child(if enabled { IconName::Check } else { IconName::X })
                .on_click(move |_, _, cx| {
                    ws2.update(cx, |this, cx| this.set_provider_enabled(id, !enabled, cx));
                }),
        )
}
