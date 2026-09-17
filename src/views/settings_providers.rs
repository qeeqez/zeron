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
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::auth::AuthState;
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
    /// Paste-back field for login flows that need a code (claude).
    pub login_code: Entity<InputState>,
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
        self.test_state.retain(|id, _| ids.contains(id.as_str()));
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
        let login_code = cx.new(|cx| InputState::new(window, cx).placeholder("Paste code…"));
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
        ProviderInputs { name, command, key_env, login_code }
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
        .child(
            div()
                .flex()
                .items_center()
                .child(group_label("Model providers", &s.search, cx))
                .child(div().flex_1())
                .child(crate::views::settings_provider_health::refresh_button(s))
                .child(Button::new("provider-add").label("Add provider").icon(IconName::Plus).small().outline().on_click({
                    let panel = s.panel.clone();
                    move |_, window, cx| panel.update(cx, |this, cx| this.open_provider_wizard(window, cx))
                })),
        )
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
    let (id_sel, id_t, id_in) = (p.id.clone(), p.id.clone(), p.id.clone());
    let (panel, ws_t, ws_in) = (s.panel.clone(), s.ws.clone(), s.ws.clone());
    let auth = s.ws.read(cx).auth_state(&p.id);
    let status = if !p.enabled {
        "Disabled".to_string()
    } else {
        auth.row_status().unwrap_or_else(|| p.kind.info().tagline.to_string())
    };
    let status_color = match auth {
        AuthState::SignedIn(_) => cx.theme().success,
        AuthState::SignedOut if p.enabled => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
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
        .child(crate::views::settings_provider_health::health_dot(p, s, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_xs().font_semibold().overflow_hidden().child(p.name.clone()))
                .child(div().text_xs().text_color(status_color).child(status))
                .children(crate::views::settings_provider_test::test_result(p, s, cx)),
        )
        .child(div().flex_1())
        .child(crate::views::settings_provider_test::test_button(p, s))
        .when(matches!(auth, AuthState::SignedOut) && p.enabled && crate::auth::can_sign_in(p.kind), |d| {
            d.child(
                Button::new(SharedString::from(format!("provider-sign-in-{id_in}")))
                    .label("Sign in")
                    .icon(IconName::LogIn)
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| ws_in.update(cx, |this, cx| this.sign_in(&id_in, cx))),
            )
        })
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

/// The paste-back row for flows that need a code (claude): the input plus
/// a Submit button that writes it to the login child's stdin.
pub(crate) fn code_row(id: &str, inputs: &ProviderInputs, ws: &Entity<Workspace>) -> impl IntoElement {
    let code_input = inputs.login_code.clone();
    let (id, ws) = (id.to_string(), ws.clone());
    div()
        .flex()
        .gap_2()
        .items_center()
        .child(Input::new(&inputs.login_code).id(SharedString::from(format!("auth-code-{id}"))).appearance(true))
        .child(
            Button::new(SharedString::from(format!("auth-submit-{id}")))
                .label("Submit")
                .small()
                .outline()
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| {
                        let code = code_input.read(cx).value().to_string();
                        this.submit_auth_code(&id, &code, cx);
                    });
                }),
        )
}
