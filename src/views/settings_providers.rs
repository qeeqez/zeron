//! Providers settings section: a master-detail layout — a scrollable list of
//! provider instances on the left (icon, name, status, enable switch), the
//! selected instance's detail panel on the right (see
//! `settings_provider_detail`), and an "Add provider" button that opens the
//! wizard in `settings_provider_wizard`. Rows read the workspace directly so
//! toggles re-render in place.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::providers::ProviderInstance;
use crate::views::settings::SettingsPanel;
use crate::views::settings_provider_detail::detail_panel;
use crate::views::settings_sections::{SettingsView, group_label};
use crate::workspace::Workspace;

/// The editable fields one instance exposes in the detail panel — the
/// `InputState` entities live on `SettingsPanel` so typed text survives
/// re-renders; `SettingsView` carries a clone for the section body.
#[derive(Clone)]
pub(crate) struct ProviderInputs {
    pub name: Entity<InputState>,
    pub command: Entity<InputState>,
    pub key_env: Entity<InputState>,
}

/// Which connection field an input writes — bundled with the instance id so
/// the subscribe closure stays under the argument-count lint.
#[derive(Clone)]
struct FieldCtx {
    field: ProviderField,
    id: String,
    ws: WeakEntity<Workspace>,
}

#[derive(Clone, Copy)]
enum ProviderField {
    Name,
    Command,
    KeyEnv,
}

/// Write a changed detail-panel field onto its instance — name goes through
/// `rename_provider`, connection fields through `configure_provider` (which
/// rebuilds the backend when it's the selected one).
fn on_provider_field(ctx: &FieldCtx, state: &Entity<InputState>, event: &InputEvent, cx: &mut App) {
    if !matches!(event, InputEvent::Change) {
        return;
    }
    let value = state.read(cx).value().to_string();
    let _ = ctx.ws.update(cx, |this, cx| match ctx.field {
        ProviderField::Name => this.rename_provider(&ctx.id, value, cx),
        ProviderField::Command | ProviderField::KeyEnv => {
            let Some(p) = this.providers.iter().find(|p| p.id == ctx.id) else { return };
            let (command, key_env) = match ctx.field {
                ProviderField::Command => (value, p.key_env.clone()),
                _ => (p.command.clone(), value),
            };
            this.configure_provider(&ctx.id, command, key_env);
            cx.notify();
        },
    });
}

impl SettingsPanel {
    /// Reconcile the per-instance input map with the live instance list:
    /// create + subscribe inputs for new instances, drop entries for removed
    /// ones (dropping the subscriptions too), and keep `provider_selection`
    /// pointing at a real instance.
    pub(crate) fn sync_provider_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let instances: Vec<ProviderInstance> = self.ws.upgrade().map(|ws| ws.read(cx).provider_instances().to_vec()).unwrap_or_default();
        let ids: std::collections::HashSet<&str> = instances.iter().map(|p| p.id.as_str()).collect();
        self.provider_inputs.retain(|id, _| ids.contains(id.as_str()));
        for p in &instances {
            if !self.provider_inputs.contains_key(&p.id) {
                let inputs = self.new_provider_inputs(p, window, cx);
                self.provider_inputs.insert(p.id.clone(), inputs);
            }
        }
        if self.provider_selection.as_deref().is_none_or(|id| !ids.contains(id)) {
            self.provider_selection = instances.first().map(|p| p.id.clone());
        }
    }

    /// Create the three detail inputs for one instance, seeded from its
    /// persisted fields and subscribed to write edits back.
    fn new_provider_inputs(&mut self, p: &ProviderInstance, window: &mut Window, cx: &mut Context<Self>) -> ProviderInputs {
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Display name");
            s.set_value(p.name.clone(), window, cx);
            s
        });
        let command = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Command");
            s.set_value(p.command.clone(), window, cx);
            s
        });
        let key_env = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("ENV_VAR_NAME");
            s.set_value(p.key_env.clone(), window, cx);
            s
        });
        // Persist on every edit — the backend reads these at send time.
        for (input, field) in [
            (name.clone(), ProviderField::Name),
            (command.clone(), ProviderField::Command),
            (key_env.clone(), ProviderField::KeyEnv),
        ] {
            let ctx = FieldCtx { field, id: p.id.clone(), ws: self.ws.clone() };
            cx.subscribe_in(&input, window, move |_, state, event: &InputEvent, _window, cx| {
                on_provider_field(&ctx, state, event, cx);
            })
            .detach();
        }
        ProviderInputs { name, command, key_env }
    }
}

/// The Providers content pane: header + add button, then the master-detail
/// split — scrollable instance list left, selected instance's detail right.
pub fn providers_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let instances = s.ws.read(cx).provider_instances().to_vec();
    let selected = s.provider_selection.as_deref().and_then(|id| instances.iter().find(|p| p.id == id).cloned());
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().flex().items_center().child(group_label("Model providers", cx)).child(div().flex_1()).child(
            Button::new("provider-add").label("Add provider").icon(IconName::Plus).small().outline().on_click({
                let panel = s.panel.clone();
                move |_, window, cx| panel.update(cx, |this, cx| this.open_provider_wizard(window, cx))
            }),
        ))
        .child(
            div()
                .id("providers-split")
                .test_support()
                .flex()
                .gap_3()
                .h(px(420.))
                .child(instance_list(&instances, s, cx))
                .child(detail_panel(selected.as_ref(), s, cx)),
        )
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
}

/// The scrollable left column: one selectable row per instance with the
/// kind icon, name, status line and an enable `Switch`.
fn instance_list(instances: &[ProviderInstance], s: &SettingsView, cx: &App) -> impl IntoElement {
    let mut list = div()
        .id("provider-list")
        .test_support()
        .w(px(220.))
        .flex_shrink_0()
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_1();
    if instances.is_empty() {
        list = list.child(
            div()
                .p_3()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No providers yet — add one to get started."),
        );
    }
    list.children(instances.iter().map(|p| instance_row(p, s, cx)))
}

/// One instance row: click selects it for the detail panel; the switch
/// toggles `enabled` without disturbing the selection.
fn instance_row(p: &ProviderInstance, s: &SettingsView, cx: &App) -> impl IntoElement {
    let selected = s.provider_selection.as_deref() == Some(p.id.as_str());
    let (id_sel, id_t) = (p.id.clone(), p.id.clone());
    let (panel, ws_t) = (s.panel.clone(), s.ws.clone());
    div()
        .id(SharedString::from(format!("provider-row-{}", p.id)))
        .test_support()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .p_2()
        .rounded_md()
        .when(selected, |d| d.bg(cx.theme().accent.opacity(0.15)))
        .child(div().text_color(cx.theme().muted_foreground).child(p.kind.info().icon))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_xs().font_semibold().overflow_hidden().child(p.name.clone()))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if p.enabled {
                    p.kind.info().tagline
                } else {
                    "Disabled"
                })),
        )
        .child(div().flex_1())
        .child(
            Switch::new(SharedString::from(format!("provider-enable-{}", p.id)))
                .checked(p.enabled)
                .small()
                .accessibility_label(format!("Enable {}", p.name))
                .on_click(move |on, _, cx| {
                    ws_t.update(cx, |this, cx| this.set_provider_enabled(&id_t, *on, cx));
                }),
        )
        .on_click(move |_, _, cx| {
            panel.update(cx, |this, cx| {
                this.provider_selection = Some(id_sel.clone());
                cx.notify();
            });
        })
}
