//! Providers settings section: add/remove/enable provider instances,
//! per-model enable + ordering, and the HTTP transport inputs. Rows read
//! the workspace directly so toggles re-render in place.
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::providers::{ProviderInstance, ProviderKind};
use crate::views::settings_sections::{SettingsView, group_label};
use crate::workspace::Workspace;

/// The Providers content pane: one block per configured instance (toggle,
/// remove, per-model rows) plus an add row and the HTTP endpoint inputs.
pub fn providers_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let instances = s.ws.read(cx).provider_instances().to_vec();
    let http = instances.iter().find(|p| p.kind == ProviderKind::Http).cloned();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Model providers", cx))
        .children(instances.iter().map(|p| provider_block(p, &s.ws, cx)))
        .child(div().flex().gap_2().children(ProviderKind::ALL.into_iter().map(|kind| {
            let ws = s.ws.clone();
            div()
                .id(SharedString::from(format!("provider-add-{}", kind.slug())))
                .test_support()
                .cursor_pointer()
                .text_xs()
                .text_color(cx.theme().accent)
                .child(format!("+ {}", kind.info().label))
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| {
                        this.add_provider(kind, kind.info().label.to_string(), cx);
                    });
                })
        })))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Disabled providers are hidden from the model picker."),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("Active backend: {}", s.backend)),
        )
        .when_some(http, |d, _| {
            d.child(group_label("HTTP endpoint", cx)).child(
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
        })
}

/// One instance block: name + tagline + catalog size, an "active" marker,
/// enable/remove controls, then one toggle row per catalog model.
fn provider_block(p: &ProviderInstance, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let (active, models) = ws.read_with(cx, |w, _| (w.selected_provider() == Some(p.id.as_str()), w.models_config_for(&p.id)));
    let enabled = p.enabled;
    let id = p.id.clone();
    let ws_toggle = ws.clone();
    let ws_remove = ws.clone();
    let id_remove = id.clone();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(div().font_semibold().child(p.name.clone()))
                .child(div().text_color(cx.theme().muted_foreground).child(p.kind.info().tagline))
                .child(div().text_color(cx.theme().muted_foreground).child(format!("· {} models", models.len())))
                .when(active, |d| d.child(div().text_color(cx.theme().accent).child("· active")))
                .child(div().flex_1())
                .child(icon_btn(&format!("provider-toggle-{id}"), if enabled { IconName::Check } else { IconName::X }, move |_, _, cx| {
                    ws_toggle.update(cx, |this, cx| this.set_provider_enabled(&id, !enabled, cx));
                }))
                .child(icon_btn(&format!("provider-remove-{id_remove}"), IconName::Trash, move |_, _, cx| {
                    ws_remove.update(cx, |this, cx| this.remove_provider(&id_remove, cx));
                })),
        )
        .children(models.into_iter().map(|(m, on)| model_row(&p.id, m, on, ws, cx)))
}

/// One model row inside an instance block: enable toggle, id, and
/// up/down reorder controls.
fn model_row(pid: &str, m: crate::model::ModelInfo, on: bool, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let pid = pid.to_string();
    let mid = m.id.to_string();
    let (ws_t, ws_up, ws_dn) = (ws.clone(), ws.clone(), ws.clone());
    let (id_t, id_up, id_dn) = (mid.clone(), mid.clone(), mid.clone());
    let (p_t, p_up, p_dn) = (pid.clone(), pid.clone(), pid.clone());
    div()
        .flex()
        .items_center()
        .gap_2()
        .pl_4()
        .text_xs()
        .child(icon_btn(&format!("model-toggle-{pid}-{mid}"), if on { IconName::Check } else { IconName::X }, move |_, _, cx| {
            ws_t.update(cx, |this, cx| this.set_model_enabled(&p_t, &id_t, !on, cx));
        }))
        .child(div().text_color(cx.theme().muted_foreground).child(m.label.clone()))
        .child(div().flex_1())
        .child(icon_btn(&format!("model-up-{pid}-{id_up}"), IconName::ChevronUp, move |_, _, cx| {
            ws_up.update(cx, |this, cx| this.move_model(&p_up, &id_up, -1, cx));
        }))
        .child(icon_btn(&format!("model-down-{pid}-{id_dn}"), IconName::ChevronDown, move |_, _, cx| {
            ws_dn.update(cx, |this, cx| this.move_model(&p_dn, &id_dn, 1, cx));
        }))
}

/// A small clickable icon — the shared shape for toggle/remove/reorder.
fn icon_btn(id: &str, icon: IconName, on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .test_support()
        .cursor_pointer()
        .child(icon)
        .on_click(on_click)
}
