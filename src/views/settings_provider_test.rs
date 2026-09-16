//! "Test connection" for provider instances: a per-row button that runs
//! the instance's catalog fetch (`model_catalog::probe_instance`) on the
//! background executor and reports the outcome inline — spinner while it
//! runs, "N models · Xms" on success, the error's first line (full text in
//! the tooltip) on failure. The wizard's Config step gets the same probe
//! against the not-yet-saved draft. Split from `settings_providers` to
//! stay under the SLOC cap.

use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::ModelInfo;
use crate::providers::ProviderInstance;
use crate::views::settings::SettingsPanel;

/// One instance's probe outcome — runtime-only state on `SettingsPanel`
/// (`test_state`), never persisted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum TestState {
    #[default]
    Idle,
    /// A probe is in flight — the button is inert until it lands.
    Testing,
    /// `models` is the fetched catalog size, `ms` the probe latency.
    Ok { models: usize, ms: u64 },
    /// The probe's error — the row shows its first line.
    Err(String),
}

impl SettingsPanel {
    /// Start a probe for `instance_id`. No-op while one runs (the button is
    /// disabled anyway — this guards the keyboard path). Test builds skip
    /// the spawn: tests land outcomes via `land_provider_test` instead of
    /// running a real backend.
    pub(crate) fn test_provider(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        if matches!(self.test_state.get(instance_id), Some(TestState::Testing)) {
            return;
        }
        let Some(p) = self
            .ws
            .upgrade()
            .and_then(|ws| ws.read(cx).provider_instances().iter().find(|p| p.id == instance_id).cloned())
        else {
            return;
        };
        self.test_state.insert(instance_id.to_string(), TestState::Testing);
        cx.notify();
        if cfg!(test) {
            return;
        }
        let task = cx.background_executor().spawn(async move {
            let started = Instant::now();
            (crate::model_catalog::probe_instance(&p), started.elapsed())
        });
        let id = instance_id.to_string();
        cx.spawn(async move |this, cx| {
            let (result, elapsed) = task.await;
            let _ = this.update(cx, |this, cx| this.land_provider_test(&id, result, elapsed.as_millis() as u64, cx));
        })
        .detach();
    }

    /// Publish a probe outcome. A fetched catalog also lands on the
    /// workspace — a successful test doubles as a catalog refresh.
    pub(crate) fn land_provider_test(
        &mut self, instance_id: &str, result: Result<Vec<ModelInfo>, String>, ms: u64, cx: &mut Context<Self>,
    ) {
        self.test_state.insert(
            instance_id.to_string(),
            match &result {
                Ok(models) => TestState::Ok { models: models.len(), ms },
                Err(e) => TestState::Err(e.clone()),
            },
        );
        // Skip the catalog land when the instance was removed mid-probe —
        // a stale entry would resurrect its picker list + cache file.
        if let Ok(models) = result
            && let Some(ws) = self.ws.upgrade()
            && ws.read(cx).provider_instances().iter().any(|p| p.id == instance_id)
        {
            ws.update(cx, |this, cx| this.land_catalog(instance_id, models, cx));
        }
        cx.notify();
    }

    /// Probe the wizard's draft instance — the Config step's fields before
    /// "Add provider" saves them. Same spawn seam as `test_provider`.
    pub(crate) fn test_wizard_provider(&mut self, cx: &mut Context<Self>) {
        let Some(w) = self.provider_wizard.as_mut() else { return };
        if matches!(w.test_state, TestState::Testing) {
            return;
        }
        let p = w.draft_instance(cx);
        w.test_state = TestState::Testing;
        cx.notify();
        if cfg!(test) {
            return;
        }
        let task = cx.background_executor().spawn(async move {
            let started = Instant::now();
            (crate::model_catalog::probe_instance(&p), started.elapsed())
        });
        cx.spawn(async move |this, cx| {
            let (result, elapsed) = task.await;
            let _ = this.update(cx, |this, cx| this.land_wizard_test(result, elapsed.as_millis() as u64, cx));
        })
        .detach();
    }

    /// Publish the wizard probe outcome — ignored when the dialog closed
    /// mid-probe.
    pub(crate) fn land_wizard_test(&mut self, result: Result<Vec<ModelInfo>, String>, ms: u64, cx: &mut Context<Self>) {
        if let Some(w) = self.provider_wizard.as_mut() {
            w.test_state = match result {
                Ok(models) => TestState::Ok { models: models.len(), ms },
                Err(e) => TestState::Err(e),
            };
        }
        cx.notify();
    }
}

/// The row's "Test" button — a spinner while the probe runs, inert to
/// clicks until it lands.
pub(crate) fn test_button(p: &ProviderInstance, s: &crate::views::settings_sections::SettingsView) -> impl IntoElement {
    let testing = matches!(s.test_state.get(&p.id), Some(TestState::Testing));
    let (id, panel) = (p.id.clone(), s.panel.clone());
    Button::new(SharedString::from(format!("provider-test-{id}")))
        .label(if testing { "Testing…" } else { "Test" })
        .icon(IconName::Zap)
        .small()
        .ghost()
        .disabled(testing)
        .loading(testing)
        .on_click(move |_, _, cx| panel.update(cx, |this, cx| this.test_provider(&id, cx)))
}

/// The probe outcome line under the row's status — nothing while idle or
/// testing (the button's spinner covers that), then the result. The error
/// line carries the full message as its tooltip and accessibility label.
pub(crate) fn test_result(p: &ProviderInstance, s: &crate::views::settings_sections::SettingsView, cx: &App) -> Option<AnyElement> {
    let (icon, color, text, tip) = match s.test_state.get(&p.id)? {
        TestState::Ok { models, ms } => (IconName::CircleCheck, cx.theme().success, format!("{models} models · {ms}ms"), None),
        TestState::Err(e) => (IconName::CircleX, cx.theme().danger, e.lines().next().unwrap_or_default().to_string(), Some(e.clone())),
        _ => return None,
    };
    let mut d = div()
        .id(SharedString::from(format!("provider-test-result-{}", p.id)))
        .test_support()
        .aria_label(tip.clone().unwrap_or_else(|| text.clone()))
        .flex()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(color)
        .child(icon)
        .child(text);
    if let Some(tip) = tip {
        d = d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
    }
    Some(d.into_any_element())
}

/// The wizard Config step's test row — same button + outcome against the
/// draft instance, so a bad command/key fails before the provider exists.
pub(crate) fn wizard_test_row(panel: &Entity<SettingsPanel>, cx: &App) -> impl IntoElement {
    let state = panel.read(cx).provider_wizard.as_ref().map_or(TestState::Idle, |w| w.test_state.clone());
    let testing = matches!(state, TestState::Testing);
    let mut d = div().flex().items_center().gap_2().child(
        Button::new("wizard-test")
            .label(if testing { "Testing…" } else { "Test connection" })
            .icon(IconName::Zap)
            .small()
            .outline()
            .disabled(testing)
            .loading(testing)
            .on_click({
                let panel = panel.clone();
                move |_, _, cx| panel.update(cx, |this, cx| this.test_wizard_provider(cx))
            }),
    );
    match state {
        TestState::Ok { models, ms } => {
            d = d.child(
                div()
                    .id("wizard-test-result")
                    .test_support()
                    .aria_label(format!("{models} models · {ms}ms"))
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().success)
                    .child(IconName::CircleCheck)
                    .child(format!("{models} models · {ms}ms")),
            );
        },
        TestState::Err(e) => {
            let tip = e.clone();
            d = d.child(
                div()
                    .id("wizard-test-result")
                    .test_support()
                    .aria_label(e.clone())
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(IconName::CircleX)
                    .child(e.lines().next().unwrap_or_default().to_string())
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
            );
        },
        _ => {},
    }
    d
}
