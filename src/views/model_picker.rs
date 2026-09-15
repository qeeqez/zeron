//! The composer's two-pane model picker: a `Popover` whose left column
//! lists the enabled provider instances (kind icon + name) and whose right
//! column lists the browsed instance's models. Clicking a provider browses
//! it; clicking a model selects instance+model and dismisses the popover.
//! There is no synthetic default entry — an instance with no catalog shows
//! an empty right pane.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::ModelInfo;
use crate::workspace::Workspace;

/// One enabled provider instance as the picker's left pane sees it.
pub struct PickerProvider {
    pub id: String,
    pub name: String,
    /// The instance kind's icon — `ProviderKindInfo::icon`.
    pub icon: IconName,
    /// The instance's effective model list — `Workspace::models_for`.
    pub models: Vec<ModelInfo>,
}

/// Owned inputs for `model_picker` — the composer builds this from
/// `&Workspace` so the returned element holds no borrow.
pub struct ModelPickerSpec {
    /// Selected instance id — empty when no instance exists.
    pub current_provider: String,
    pub current_model: SharedString,
    /// Enabled instances in user order.
    pub providers: Vec<PickerProvider>,
    pub ws: Entity<Workspace>,
}

/// The picker button: label is "provider · model", content is the two
/// panes.
pub fn model_picker(spec: ModelPickerSpec) -> impl IntoElement {
    let provider_label = spec
        .providers
        .iter()
        .find(|p| p.id == spec.current_provider)
        .map_or_else(|| spec.current_provider.clone(), |p| p.name.clone());
    let model_label = spec
        .providers
        .iter()
        .find(|p| p.id == spec.current_provider)
        .and_then(|p| p.models.iter().find(|m| m.id == spec.current_model))
        .map_or_else(|| spec.current_model.to_string(), |m| m.label.to_string());
    let label = format!("{provider_label} · {model_label}");
    Popover::new("model-picker")
        .trigger(Button::new("model").ghost().label(label).icon(IconName::ChevronsUpDown))
        .content(move |_, window, cx| panes(&spec, window, cx))
}

/// The two columns. `browse` is keyed element state: it lives only while
/// the popover is open, so reopening always lands on the active provider.
fn panes(spec: &ModelPickerSpec, window: &mut Window, cx: &mut Context<PopoverState>) -> AnyElement {
    let browse = window.use_keyed_state("model-picker-browse", cx, |_, _| spec.current_provider.clone());
    let browsed_id = browse.read(cx).clone();
    let browsed = spec.providers.iter().find(|p| p.id == browsed_id).or_else(|| spec.providers.first());
    h_flex()
        .id("model-picker-panes")
        .test_support()
        .gap_1()
        .child(provider_pane(spec, &browsed_id, &browse, cx))
        .child(model_pane(spec, browsed, cx.entity(), cx))
        .into_any_element()
}

/// LEFT: one row per enabled instance — kind icon, name, a check on the
/// active instance, accent background on the browsed one. Click browses.
fn provider_pane(spec: &ModelPickerSpec, browsed_id: &str, browse: &Entity<String>, cx: &App) -> impl IntoElement {
    let rows = spec
        .providers
        .iter()
        .map(|p| provider_row(p, p.id == spec.current_provider, p.id == browsed_id, browse, cx))
        .collect::<Vec<_>>();
    v_flex()
        .id("model-picker-providers")
        .test_support()
        .w(px(180.))
        .max_h(px(320.))
        .overflow_y_scroll()
        .gap_0p5()
        .children(rows)
}

/// RIGHT: the browsed instance's models, or an empty state when it has no
/// catalog. Clicking a model selects it on the workspace and dismisses.
fn model_pane(spec: &ModelPickerSpec, browsed: Option<&PickerProvider>, popover: Entity<PopoverState>, cx: &App) -> impl IntoElement {
    let pane = v_flex()
        .id("model-picker-models")
        .test_support()
        .w(px(260.))
        .max_h(px(320.))
        .overflow_y_scroll()
        .gap_0p5();
    let Some(provider) = browsed else {
        return pane.child(empty_row("No providers", cx));
    };
    if provider.models.is_empty() {
        return pane.child(empty_row("No models", cx));
    }
    let ctx = ModelRowCtx {
        ws: spec.ws.clone(),
        popover,
        provider: provider.id.clone(),
        current_provider: spec.current_provider.clone(),
        current_model: spec.current_model.clone(),
    };
    pane.children(provider.models.iter().map(|m| model_row(&ctx, m, cx)).collect::<Vec<_>>())
}

/// One provider row in the left pane.
fn provider_row(p: &PickerProvider, active: bool, browsed: bool, browse: &Entity<String>, cx: &App) -> impl IntoElement {
    let id = p.id.clone();
    let browse = browse.clone();
    h_flex()
        .id(SharedString::from(format!("provider-{}", p.id)))
        .test_support()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .when(browsed, |d| d.bg(cx.theme().accent))
        .hover(|d| d.bg(cx.theme().accent))
        .child(p.icon)
        .child(div().flex_1().min_w_0().whitespace_nowrap().text_ellipsis().child(p.name.clone()))
        .when(active, |d| d.child(IconName::Check))
        .on_click(move |_, _, cx| {
            browse.update(cx, |b, _| *b = id.clone());
        })
}

/// Everything one model row needs — bundled to stay under the arg-count
/// lint.
struct ModelRowCtx {
    ws: Entity<Workspace>,
    popover: Entity<PopoverState>,
    /// The instance whose models this pane lists.
    provider: String,
    current_provider: String,
    current_model: SharedString,
}

/// One model row in the right pane: label + dimmed description, a check on
/// the selected model, click selects instance+model and closes the popover.
fn model_row(ctx: &ModelRowCtx, m: &ModelInfo, cx: &App) -> impl IntoElement {
    let selected = ctx.current_provider == ctx.provider && ctx.current_model.as_ref() == m.id.as_ref();
    let ws = ctx.ws.clone();
    let popover = ctx.popover.clone();
    let pid = ctx.provider.clone();
    let mid = m.id.clone();
    h_flex()
        .id(SharedString::from(format!("model-opt-{}-{}", ctx.provider, m.id)))
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
                this.select_model(&pid, &mid, cx);
            });
            popover.update(cx, |state, cx| state.dismiss(window, cx));
        })
}

/// The right pane's placeholder when the browsed instance has no catalog.
fn empty_row(text: &str, cx: &App) -> impl IntoElement {
    div()
        .id("model-picker-empty")
        .test_support()
        .px_2()
        .py_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.to_string())
}

/// Owned inputs for the composer's small option pickers (mode, effort) —
/// the button shows `current`, the menu lists `options` with a check on
/// the current one, and `set` applies a pick on the workspace.
pub struct PickerSpec {
    pub id: &'static str,
    pub current: SharedString,
    pub options: &'static [&'static str],
    pub ws: Entity<Workspace>,
    pub set: fn(&mut Workspace, &'static str),
}

/// A ghost button opening a dropdown of static options — the mode
/// picker's shape.
pub fn picker(spec: PickerSpec) -> impl IntoElement {
    let PickerSpec { id, current, options, ws, set } = spec;
    Button::new(id)
        .ghost()
        .label(current.clone())
        .icon(IconName::ChevronsUpDown)
        .dropdown_menu(move |menu, _window, _cx| {
            options.iter().fold(menu, |menu, opt| {
                let ws = ws.clone();
                let checked = *opt == current.as_str();
                menu.item(PopupMenuItem::new(*opt).checked(checked).on_click(move |_, _, cx| {
                    crate::views::apply_pick(&ws, set, opt, cx);
                }))
            })
        })
}

/// Owned inputs for `effort_picker` — the composer builds this from
/// `&Workspace` so the returned element holds no borrow.
pub struct EffortPickerSpec {
    /// The thread's explicit pick — `None` follows `default_effort`.
    pub current: Option<String>,
    /// The selected model's `defaultReasoningEffort`; may be empty.
    pub default_effort: String,
    /// The selected model's `supportedReasoningEfforts`.
    pub options: Vec<String>,
    pub ws: Entity<Workspace>,
}

/// The reasoning-effort dropdown beside the model picker: one item per
/// advertised effort, a check on the effective one, and a "Default" row
/// that clears the override back to the model's `defaultReasoningEffort`.
pub fn effort_picker(spec: EffortPickerSpec) -> impl IntoElement {
    let effective = spec.current.clone().unwrap_or_else(|| spec.default_effort.clone());
    Button::new("effort")
        .ghost()
        .label(if effective.is_empty() { "Effort".to_string() } else { effective.clone() })
        .icon(IconName::ChevronsUpDown)
        .dropdown_menu(move |menu, _window, _cx| {
            let default_label = if spec.default_effort.is_empty() {
                "Default".to_string()
            } else {
                format!("Default ({})", spec.default_effort)
            };
            let ws = spec.ws.clone();
            let menu = menu
                .item(PopupMenuItem::new(default_label).checked(spec.current.is_none()).on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.set_effort(None, cx));
                }))
                .separator();
            spec.options.iter().fold(menu, |menu, opt| {
                let ws = spec.ws.clone();
                let value = opt.clone();
                let checked = spec.current.as_deref() == Some(opt.as_str());
                menu.item(PopupMenuItem::new(opt.clone()).checked(checked).on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.set_effort(Some(&value), cx));
                }))
            })
        })
}
