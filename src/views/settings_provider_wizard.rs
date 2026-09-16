//! The "Add provider" wizard: a three-step dialog (Driver → Identity →
//! Config) opened from the Providers settings section. State lives on
//! `SettingsPanel::provider_wizard` so the dialog's per-frame builder can
//! read it; inputs are fresh entities per open. Step bodies live in
//! `settings_provider_wizard_steps`.

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::provider_ops::ProviderDraft;
use crate::providers::ProviderKind;
use crate::views::settings::SettingsPanel;
use crate::views::settings_provider_wizard_steps::{config_step, driver_step, identity_step};
use crate::workspace::Workspace;

/// Wizard steps in order — the pill row renders from this.
const STEPS: [&str; 3] = ["Driver", "Identity", "Config"];

/// In-flight wizard state for one "Add provider" dialog.
pub(crate) struct ProviderWizard {
    /// Index into `STEPS`.
    pub step: usize,
    pub kind: ProviderKind,
    pub label: Entity<InputState>,
    pub instance_id: Entity<InputState>,
    pub command: Entity<InputState>,
    pub key_env: Entity<InputState>,
    pub accent: Option<String>,
    /// The Config step's "Test connection" outcome against the draft.
    pub test_state: crate::views::settings_provider_test::TestState,
    /// Shown when the finish step rejects the instance id.
    pub error: Option<String>,
}

impl ProviderWizard {
    /// Fresh inputs per open; `instance_id` seeds with the kind's slug,
    /// de-duplicated against existing instances.
    fn new(ws: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) -> Self {
        let kind = ProviderKind::CodexCli;
        let id = ws.upgrade().map(|ws| ws.read(cx).next_instance_id(kind)).unwrap_or_else(|| kind.slug().to_string());
        let label = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Work"));
        let instance_id = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("instance-id");
            s.set_value(id, window, cx);
            s
        });
        let command = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Command");
            s.set_value(kind.default_command(), window, cx);
            s
        });
        let key_env = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("ENV_VAR_NAME");
            s.set_value(kind.default_key_env(), window, cx);
            s
        });
        Self {
            step: 0,
            kind,
            label,
            instance_id,
            command,
            key_env,
            accent: None,
            test_state: crate::views::settings_provider_test::TestState::Idle,
            error: None,
        }
    }

    /// The instance the wizard would create right now — the Config step's
    /// "Test connection" probes this draft before it exists.
    pub(crate) fn draft_instance(&self, cx: &App) -> crate::providers::ProviderInstance {
        let mut p = crate::providers::ProviderInstance::new(self.kind, String::new());
        p.command = self.command.read(cx).value().to_string();
        p.key_env = self.key_env.read(cx).value().to_string();
        p
    }
}

impl SettingsPanel {
    /// Open the wizard dialog with fresh state.
    pub(crate) fn open_provider_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.provider_wizard = Some(ProviderWizard::new(&self.ws, window, cx));
        let panel = cx.entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            dialog
                .overlay_closable(false)
                .w(px(560.))
                .child(wizard_body(&panel, window, cx))
                .footer(wizard_footer(&panel, cx))
                .on_close({
                    let panel = panel.clone();
                    move |_, _, cx| close_wizard(&panel, cx)
                })
        });
    }

    /// Re-point the wizard at `kind`: re-seed the instance id and the
    /// connection defaults so the Identity/Config steps match the driver.
    pub(crate) fn reseed_wizard(&mut self, kind: ProviderKind, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .ws
            .upgrade()
            .map(|ws| ws.read(cx).next_instance_id(kind))
            .unwrap_or_else(|| kind.slug().to_string());
        if let Some(w) = self.provider_wizard.as_mut() {
            w.kind = kind;
            w.error = None;
            w.test_state = crate::views::settings_provider_test::TestState::Idle;
            w.instance_id.update(cx, |s, cx| s.set_value(id, window, cx));
            w.command.update(cx, |s, cx| s.set_value(kind.default_command(), window, cx));
            w.key_env.update(cx, |s, cx| s.set_value(kind.default_key_env(), window, cx));
        }
        cx.notify();
    }

    /// Step 1→2→3. The finish step owns the id check so a bad id keeps the
    /// dialog open with an error.
    fn advance_wizard(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = self.provider_wizard.as_mut() {
            w.step = (w.step + 1).min(STEPS.len() - 1);
            w.error = None;
        }
        cx.notify();
    }

    /// Create the instance from the wizard fields, land its connection
    /// config, select it in the list, and close. A rejected id stays on the
    /// Config step with the error under the body.
    fn finish_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(w) = self.provider_wizard.as_ref() else { return };
        let (kind, accent) = (w.kind, w.accent.clone());
        let name = w.label.read(cx).value().trim().to_string();
        let name = if name.is_empty() { kind.info().label.to_string() } else { name };
        let draft = ProviderDraft { name, id: w.instance_id.read(cx).value().to_string(), accent };
        let command = w.command.read(cx).value().to_string();
        let key_env = w.key_env.read(cx).value().to_string();
        let Some(ws) = self.ws.upgrade() else { return };
        // The default id + no accent is exactly what `add_provider` produces —
        // the simple path stays the common one.
        let default = draft.accent.is_none() && draft.id == ws.read(cx).next_instance_id(kind);
        let added = ws.update(cx, |this, cx| {
            let added = if default {
                Some(this.add_provider(kind, draft.name, cx))
            } else {
                this.add_provider_with_id(kind, draft, cx)
            };
            added.inspect(|id| this.configure_provider(id, command, key_env))
        });
        match added {
            Some(id) => {
                self.provider_selection = Some(id);
                self.provider_wizard = None;
                cx.notify();
                window.close_dialog(cx);
            },

            None => {
                if let Some(w) = self.provider_wizard.as_mut() {
                    w.error = Some("Instance ID is empty, taken, or has invalid characters.".to_string());
                }
                cx.notify();
            },
        }
    }
}
/// Clear the wizard state when the dialog closes by any path (X, overlay,
/// Esc) — the custom Cancel/Back buttons clear it themselves.
fn close_wizard(panel: &Entity<SettingsPanel>, cx: &mut App) {
    panel.update(cx, |this, cx| {
        this.provider_wizard = None;
        cx.notify();
    });
}

/// The step pills + current step's body — rebuilt every frame while the
/// dialog is open, so it always reflects `provider_wizard`.
fn wizard_body(panel: &Entity<SettingsPanel>, _window: &mut Window, cx: &mut App) -> impl IntoElement {
    let (step, kind, error) = panel
        .read(cx)
        .provider_wizard
        .as_ref()
        .map_or((0, ProviderKind::CodexCli, None), |w| (w.step, w.kind, w.error.clone()));
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(step_pills(step, cx))
        .child(match step {
            0 => driver_step(panel, kind, cx),
            1 => identity_step(panel, cx),
            _ => config_step(panel, kind, cx),
        })
        .when_some(error, |d, e| d.child(div().id("wizard-error").test_support().text_xs().text_color(cx.theme().danger).child(e)))
}

/// The 1-2-3 step indicator: the current step is highlighted, completed
/// steps show a check.
fn step_pills(step: usize, cx: &App) -> impl IntoElement {
    div().flex().gap_2().children(STEPS.iter().enumerate().map(|(i, name)| {
        let (current, done) = (i == step, i < step);
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded_md()
            .when(current, |d| d.bg(cx.theme().accent.opacity(0.15)))
            .child(
                div()
                    .text_xs()
                    .text_color(if done { cx.theme().accent } else { cx.theme().muted_foreground })
                    .child(if done { "✓".to_string() } else { format!("{}", i + 1) }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if current { cx.theme().foreground } else { cx.theme().muted_foreground })
                    .child(*name),
            )
    }))
}

/// Back/Cancel + Next/Add-provider buttons. Next advances steps; the last
/// step's "Add provider" calls `finish_wizard`.
fn wizard_footer(panel: &Entity<SettingsPanel>, cx: &App) -> impl IntoElement {
    let step = panel.read(cx).provider_wizard.as_ref().map_or(0, |w| w.step);
    let last = step == STEPS.len() - 1;
    let (back, next) = (panel.clone(), panel.clone());
    div()
        .flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("wizard-back")
                .label(if step == 0 { "Cancel" } else { "Back" })
                .outline()
                .on_click(move |_, window, cx| {
                    back.update(cx, |this, cx| {
                        if this.provider_wizard.as_ref().is_some_and(|w| w.step == 0) {
                            this.provider_wizard = None;
                            cx.notify();
                            window.close_dialog(cx);
                        } else if let Some(w) = this.provider_wizard.as_mut() {
                            w.step -= 1;
                            w.error = None;
                            cx.notify();
                        }
                    });
                }),
        )
        .child(
            Button::new("wizard-next")
                .label(if last { "Add provider" } else { "Next" })
                .primary()
                .on_click(move |_, window, cx| {
                    next.update(cx, |this, cx| if last { this.finish_wizard(window, cx) } else { this.advance_wizard(cx) });
                }),
        )
}
