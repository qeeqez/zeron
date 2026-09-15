//! The General section's default-model control: the composer's two-pane
//! provider→model picker shape bound to `default_model`. The composer's
//! `model_picker` itself can't be reused — its rows call `select_model`,
//! which switches the ACTIVE thread's backend; the default must leave the
//! live selection alone. Split from `settings_general.rs` to stay under the
//! 250-SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::model_picker::PickerProvider;
use crate::workspace::Workspace;

/// Owned inputs for the default-model picker — same shape as the composer's
/// `ModelPickerSpec`, but `current_*` is the EFFECTIVE default (the
/// configured default, or the live selection when none is set).
struct DefaultPickerSpec {
    current_provider: String,
    current_model: SharedString,
    providers: Vec<PickerProvider>,
    ws: Entity<Workspace>,
}

/// The default-model control: a button reading "provider · model" (or
/// "Follow current selection" when unset) that opens the same two-pane
/// provider→model popover as the composer — with `default-`-prefixed ids and
/// rows that write `default_model` instead of the active selection.
pub(crate) fn default_model_picker(ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let spec = {
        let w = ws.read(cx);
        let dm = w.default_model().clone();
        let (current_provider, current_model) = if dm.provider_instance_id.is_empty() {
            (w.selected_provider.clone(), w.model.clone())
        } else {
            (dm.provider_instance_id, dm.model_id.into())
        };
        DefaultPickerSpec {
            current_provider,
            current_model,
            providers: w
                .enabled_providers()
                .into_iter()
                .map(|p| PickerProvider {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    icon: p.kind.info().icon,
                    models: w.models_for(&p.id),
                })
                .collect(),
            ws: ws.clone(),
        }
    };
    let label = spec
        .providers
        .iter()
        .find(|p| p.id == spec.current_provider)
        .map(|p| {
            let model = p
                .models
                .iter()
                .find(|m| m.id == spec.current_model)
                .map_or_else(|| spec.current_model.to_string(), |m| m.label.to_string());
            format!("{} · {model}", p.name)
        })
        .unwrap_or_else(|| "Follow current selection".to_string());
    Popover::new("default-model-picker")
        .trigger(Button::new("default-model").ghost().label(label).icon(IconName::ChevronsUpDown))
        .content(move |_, window, cx| default_panes(&spec, window, cx))
}

/// The two columns. `browse` is keyed element state: it lives only while the
/// popover is open, so reopening always lands on the effective default's
/// provider.
fn default_panes(spec: &DefaultPickerSpec, window: &mut Window, cx: &mut Context<PopoverState>) -> AnyElement {
    let browse = window.use_keyed_state("default-model-picker-browse", cx, |_, _| spec.current_provider.clone());
    let browsed_id = browse.read(cx).clone();
    let browsed = spec.providers.iter().find(|p| p.id == browsed_id).or_else(|| spec.providers.first());
    h_flex()
        .id("default-model-picker-panes")
        .test_support()
        .gap_1()
        .child(default_provider_pane(spec, &browsed_id, &browse, cx))
        .child(default_model_pane(spec, browsed, cx.entity(), cx))
        .into_any_element()
}

/// LEFT: one row per enabled instance — kind icon, name, a check on the
/// effective default's instance, accent background on the browsed one.
fn default_provider_pane(spec: &DefaultPickerSpec, browsed_id: &str, browse: &Entity<String>, cx: &App) -> impl IntoElement {
    let rows = spec
        .providers
        .iter()
        .map(|p| {
            let id = p.id.clone();
            let browse = browse.clone();
            h_flex()
                .id(SharedString::from(format!("default-provider-{}", p.id)))
                .test_support()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(p.id == browsed_id, |d| d.bg(cx.theme().accent))
                .hover(|d| d.bg(cx.theme().accent))
                .child(p.icon)
                .child(div().flex_1().min_w_0().whitespace_nowrap().text_ellipsis().child(p.name.clone()))
                .when(p.id == spec.current_provider, |d| d.child(IconName::Check))
                .on_click(move |_, _, cx| {
                    browse.update(cx, |b, _| *b = id.clone());
                })
        })
        .collect::<Vec<_>>();
    v_flex()
        .id("default-model-picker-providers")
        .test_support()
        .w(px(180.))
        .max_h(px(320.))
        .overflow_y_scroll()
        .gap_0p5()
        .children(rows)
}

/// RIGHT: the browsed instance's models. Clicking one sets `default_model`
/// and dismisses — the active thread's selection is untouched.
fn default_model_pane(
    spec: &DefaultPickerSpec, browsed: Option<&PickerProvider>, popover: Entity<PopoverState>, cx: &App,
) -> impl IntoElement {
    let pane = v_flex()
        .id("default-model-picker-models")
        .test_support()
        .w(px(260.))
        .max_h(px(320.))
        .overflow_y_scroll()
        .gap_0p5();
    let Some(provider) = browsed else {
        return pane.child(default_empty_row("No providers", cx));
    };
    if provider.models.is_empty() {
        return pane.child(default_empty_row("No models", cx));
    }
    pane.children(
        provider
            .models
            .iter()
            .map(|m| {
                let selected = spec.current_provider == provider.id && spec.current_model.as_ref() == m.id.as_ref();
                let (ws, popover, pid, mid) = (spec.ws.clone(), popover.clone(), provider.id.clone(), m.id.clone());
                h_flex()
                    .id(SharedString::from(format!("default-model-opt-{}-{}", provider.id, m.id)))
                    .test_support()
                    .items_baseline()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|d| d.bg(cx.theme().accent))
                    .child(m.label.clone())
                    .when(!m.description.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(m.description.clone()),
                        )
                    })
                    .when(selected, |d| d.child(IconName::Check))
                    .on_click(move |_, window, cx| {
                        ws.update(cx, |this, cx| {
                            this.set_default_model(&pid, &mid, cx);
                        });
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                    })
            })
            .collect::<Vec<_>>(),
    )
}

/// The right pane's placeholder when the browsed instance has no catalog.
fn default_empty_row(text: &str, cx: &App) -> impl IntoElement {
    div()
        .id("default-model-picker-empty")
        .test_support()
        .px_2()
        .py_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.to_string())
}
