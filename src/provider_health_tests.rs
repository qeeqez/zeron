//! Provider-health tests: `probe_health` mapping over the fake probe seam
//! (cli path / daemon port / configured-or-not / signed-out), the status
//! dots the Providers section renders per level, the detail panel's reason
//! line, and the Refresh button re-probing. A submodule of
//! `settings_providers_tests` so it shares the mount/open helpers.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{SharedString, TestAppContext};

use super::{mount, open_providers};
use crate::providers::provider_detect::{HealthLevel, ProviderProbe, probe_health, set_probe};
use crate::providers::{ProviderInstance, ProviderKind};

/// A probe that answers from fixed sets — no PATH or socket I/O.
struct FakeProbe {
    bins: &'static [&'static str],
    ports: &'static [u16],
}

impl ProviderProbe for FakeProbe {
    fn cli_path(&self, bin: &str) -> Option<String> {
        self.bins.contains(&bin).then(|| format!("/fake/bin/{bin}"))
    }

    fn daemon_up(&self, port: u16) -> bool {
        self.ports.contains(&port)
    }
}

fn fake(bins: &'static [&'static str], ports: &'static [u16]) -> FakeProbe {
    FakeProbe { bins, ports }
}

fn instance(kind: ProviderKind) -> ProviderInstance {
    ProviderInstance::new(kind, kind.info().label.to_string())
}

// ---- probe → status mapping ----

#[test]
fn cli_found_maps_ready_with_path() {
    let h = probe_health(&instance(ProviderKind::ClaudeCli), false, &fake(&["claude"], &[]));
    assert_eq!(h.level, HealthLevel::Ready);
    assert_eq!(h.reason, "claude found at /fake/bin/claude");
}

#[test]
fn cli_found_signed_out_maps_degraded() {
    let h = probe_health(&instance(ProviderKind::ClaudeCli), true, &fake(&["claude"], &[]));
    assert_eq!(h.level, HealthLevel::Degraded);
    assert!(h.reason.contains("not signed in"), "reason was: {}", h.reason);
}

#[test]
fn cli_absent_maps_missing() {
    let h = probe_health(&instance(ProviderKind::CodexCli), false, &fake(&[], &[]));
    assert_eq!(h.level, HealthLevel::Missing);
    assert_eq!(h.reason, "codex not found on PATH");
}

#[test]
fn daemon_down_with_cli_maps_degraded() {
    let h = probe_health(&instance(ProviderKind::Ollama), false, &fake(&["ollama"], &[]));
    assert_eq!(h.level, HealthLevel::Degraded);
    assert_eq!(h.reason, "ollama not listening on :11434");
}

#[test]
fn daemon_up_without_cli_maps_ready() {
    // The app talks HTTP to the daemon — the CLI is optional.
    let h = probe_health(&instance(ProviderKind::Ollama), false, &fake(&[], &[11434]));
    assert_eq!(h.level, HealthLevel::Ready);
    assert_eq!(h.reason, "ollama daemon listening on :11434");
}

#[test]
fn daemon_down_without_cli_maps_missing() {
    let h = probe_health(&instance(ProviderKind::Ollama), false, &fake(&[], &[]));
    assert_eq!(h.level, HealthLevel::Missing);
    assert!(h.reason.contains("not on PATH") && h.reason.contains("11434"), "reason was: {}", h.reason);
}

#[test]
fn configured_or_not_maps_ready_and_missing() {
    // http/mcp/acp have nothing installable — a filled command is the status.
    let mut http = instance(ProviderKind::Http);
    http.command = "https://api.example.com".to_string();
    assert_eq!(probe_health(&http, false, &fake(&[], &[])).level, HealthLevel::Ready);
    http.command.clear();
    let h = probe_health(&http, false, &fake(&[], &[]));
    assert_eq!(h.level, HealthLevel::Missing);
    assert_eq!(h.reason, "not configured");
    // acp seeds a default command → configured out of the box.
    assert_eq!(probe_health(&instance(ProviderKind::Acp), false, &fake(&[], &[])).level, HealthLevel::Ready);
}

#[test]
fn sim_is_always_ready() {
    let h = probe_health(&instance(ProviderKind::Sim), false, &fake(&[], &[]));
    assert_eq!(h.level, HealthLevel::Ready);
}

// ---- dots, reason line, refresh ----

#[test]
fn rows_render_a_dot_per_probe_state() {
    let mut app = TestAppContext::single();
    set_probe(Arc::new(fake(&["claude", "ollama"], &[])));
    let (_ws, cx) = mount(&mut app);
    open_providers(cx);
    // Before the pass lands every dot is the muted "checking…" state.
    cx.update(|window, _| {
        assert!(window.try_find("provider-health-dot-sim-pending").is_some(), "pending dot before first land");
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let dot = |id: &str| window.try_find(SharedString::from(id)).is_some();
        assert!(dot("provider-health-dot-claude-cli-ready"), "cli on PATH → green");
        assert!(dot("provider-health-dot-ollama-degraded"), "cli present, daemon down → amber");
        assert!(dot("provider-health-dot-codex-cli-missing"), "cli absent → gray");
        assert!(dot("provider-health-dot-http-missing"), "unconfigured http → gray");
        assert!(dot("provider-health-dot-acp-ready"), "default command → green");
        assert!(dot("provider-health-dot-sim-ready"), "sim → green");
    });
}

#[test]
fn detail_shows_the_reason_line() {
    let mut app = TestAppContext::single();
    set_probe(Arc::new(fake(&["claude"], &[])));
    let (_ws, cx) = mount(&mut app);
    open_providers(cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("provider-row-claude-cli", cx);
        window.draw(cx).clear(cx);
        let line = window.find("provider-health-claude-cli");
        assert!(line.visible(), "detail health line renders");
        assert_eq!(line.label(), Some("claude found at /fake/bin/claude"));
    });
}

#[test]
fn refresh_reprobes_every_instance() {
    let mut app = TestAppContext::single();
    set_probe(Arc::new(fake(&["claude"], &[])));
    let (_ws, cx) = mount(&mut app);
    open_providers(cx);
    cx.run_until_parked();
    // The environment changes, then Refresh re-probes: claude leaves PATH,
    // the ollama daemon comes up without its CLI.
    set_probe(Arc::new(fake(&[], &[11434])));
    cx.update(|window, cx| {
        window.click("provider-health-refresh", cx);
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let dot = |id: &str| window.try_find(SharedString::from(id)).is_some();
        assert!(dot("provider-health-dot-claude-cli-missing"), "claude now absent → gray");
        assert!(dot("provider-health-dot-ollama-ready"), "daemon up → green");
    });
}
