//! Live provider health in Settings → Providers: a status dot on every
//! instance row (green ready / amber degraded / gray missing), a Refresh
//! button in the section header, and the same dot + one-line reason in the
//! detail panel. Probes reuse the `provider_detect` seam — `which`-style
//! PATH lookup for cli kinds, a TCP connect for daemon kinds, and
//! configured-or-not for http/mcp/acp — and run on the background executor
//! so opening settings stays instant. Split from `settings_providers` to
//! stay under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::auth::AuthState;
use crate::providers::ProviderInstance;
use crate::providers::provider_detect::{self, HealthLevel, ProviderHealth};
use crate::views::settings::SettingsPanel;
use crate::views::settings_sections::SettingsView;

impl SettingsPanel {
    /// Kick a health pass over every instance on the background executor —
    /// a no-op while one is in flight. Called when the Providers section
    /// renders without results and by the header's Refresh button;
    /// `land_provider_health` publishes the map.
    pub(crate) fn refresh_provider_health(&mut self, cx: &mut Context<Self>) {
        if self.health_pending {
            return;
        }
        let Some(ws) = self.ws.upgrade() else { return };
        let targets: Vec<(ProviderInstance, bool)> = ws
            .read(cx)
            .provider_instances()
            .iter()
            .map(|p| (p.clone(), matches!(ws.read(cx).auth_state(&p.id), AuthState::SignedOut)))
            .collect();
        self.health_pending = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move { provider_detect::scan_health(&targets) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| this.land_provider_health(found, cx));
        })
        .detach();
    }

    /// Publish a finished pass: replace the map and re-render. Instances
    /// removed mid-probe keep a stale entry until the next pass — the row
    /// is gone anyway, so the map entry is inert.
    pub(crate) fn land_provider_health(&mut self, found: Vec<(String, ProviderHealth)>, cx: &mut Context<Self>) {
        self.health_pending = false;
        self.provider_health = found.into_iter().collect();
        cx.notify();
    }
}

/// The dot's color for one level — matches the MCP status-dot palette.
fn level_color(level: HealthLevel, cx: &App) -> Hsla {
    match level {
        HealthLevel::Ready => cx.theme().success,
        HealthLevel::Degraded => cx.theme().warning,
        HealthLevel::Missing => cx.theme().muted_foreground,
    }
}

/// The row's status dot: a 6px circle colored by level, the reason as its
/// tooltip + accessibility label. Before the first pass lands (or while
/// one runs with no prior result) the dot is a muted "checking…".
pub(crate) fn health_dot(p: &ProviderInstance, s: &SettingsView, cx: &App) -> impl IntoElement {
    let (color, slug, label) = match s.provider_health.get(&p.id) {
        Some(h) => (level_color(h.level, cx), h.level.slug(), h.reason.clone()),
        None => (cx.theme().muted_foreground, "pending", "checking…".to_string()),
    };
    div()
        .id(SharedString::from(format!("provider-health-dot-{}-{slug}", p.id)))
        .test_support()
        .aria_label(label.clone())
        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
}

/// The header's Refresh button — re-probes every instance; inert while a
/// pass is in flight.
pub(crate) fn refresh_button(s: &SettingsView) -> impl IntoElement {
    let panel = s.panel.clone();
    Button::new("provider-health-refresh")
        .label("Refresh")
        .icon(IconName::RefreshCcw)
        .small()
        .ghost()
        .disabled(s.health_pending)
        .loading(s.health_pending)
        .on_click(move |_, _, cx| panel.update(cx, |this, cx| this.refresh_provider_health(cx)))
}

/// The detail panel's status line: the same dot plus the one-line reason.
pub(crate) fn health_line(p: &ProviderInstance, s: &SettingsView, cx: &App) -> impl IntoElement {
    let (color, label) = match s.provider_health.get(&p.id) {
        Some(h) => (level_color(h.level, cx), h.reason.clone()),
        None => (cx.theme().muted_foreground, "checking…".to_string()),
    };
    div()
        .id(SharedString::from(format!("provider-health-{}", p.id)))
        .test_support()
        .aria_label(label.clone())
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
        .child(div().text_xs().text_color(color).child(label))
}
