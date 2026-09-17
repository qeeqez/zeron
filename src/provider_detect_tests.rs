//! Provider-detection tests: `scan_providers` mapping over the fake probe
//! seam, the async spawn→land path on a mounted workspace, wizard
//! pre-selection of the first detected kind, and the onboarding card's
//! detection-aware button copy.

use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::providers::ProviderKind;
use crate::providers::provider_detect::{ProviderProbe, scan_providers, set_probe};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-detect-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

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

fn fake(bins: &'static [&'static str], ports: &'static [u16]) -> Arc<dyn ProviderProbe> {
    Arc::new(FakeProbe { bins, ports })
}

// ---- scan mapping ----

#[test]
fn scan_maps_clis_and_daemon_to_kinds() {
    set_probe(fake(&["codex", "claude", "ollama"], &[]));
    assert_eq!(scan_providers(), vec![ProviderKind::CodexCli, ProviderKind::ClaudeCli, ProviderKind::Ollama]);
}

#[test]
fn ollama_detects_on_daemon_alone() {
    // The daemon answering on 11434 counts even without the CLI — the app
    // talks HTTP to it.
    set_probe(fake(&[], &[11434]));
    assert_eq!(scan_providers(), vec![ProviderKind::Ollama]);
}

#[test]
fn kinds_without_probes_never_detect() {
    set_probe(fake(&["codex", "claude", "ollama", "acp", "http", "sim"], &[11434]));
    let found = scan_providers();
    for kind in [ProviderKind::Acp, ProviderKind::Http, ProviderKind::Sim] {
        assert!(!found.contains(&kind), "{kind:?} has no probe and must never detect");
    }
}

#[test]
fn empty_environment_detects_nothing() {
    set_probe(fake(&[], &[]));
    assert!(scan_providers().is_empty());
}

// ---- landing + wizard pre-selection ----

/// Drive the background scan to completion and let the landing run.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
}

#[test]
fn scan_lands_on_workspace() {
    let mut app = TestAppContext::single();
    set_probe(fake(&["claude"], &[]));
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| ws.update(cx, |w, cx| w.detect_providers(cx)));
    settle(cx);
    let detected = ws.read_with(cx, |w, _| w.detected_providers.clone());
    assert_eq!(detected, Some(vec![ProviderKind::ClaudeCli]));
    assert!(!ws.read_with(cx, |w, _| w.detection_pending));
}

#[test]
fn wizard_preselects_first_detected_kind() {
    let mut app = TestAppContext::single();
    set_probe(fake(&["claude"], &[]));
    let (ws, cx) = mount(&mut app);
    // Land a scan, then open the wizard — it seeds from the result.
    cx.update(|_, cx| ws.update(cx, |w, cx| w.detect_providers(cx)));
    settle(cx);
    cx.update(|window, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| panel.open_provider_wizard(window, cx));
    });
    settle(cx);
    let (kind, detected) = ws.read_with(cx, |w, app| {
        let w = w.settings_panel.read(app).provider_wizard.as_ref().unwrap();
        (w.kind, w.detected.clone())
    });
    assert_eq!(kind, ProviderKind::ClaudeCli, "first detected kind pre-selects");
    assert_eq!(detected, vec![ProviderKind::ClaudeCli]);
    // The seeded instance id follows the detected kind's slug.
    let id = ws.read_with(cx, |w, app| {
        w.settings_panel
            .read(app)
            .provider_wizard
            .as_ref()
            .unwrap()
            .instance_id
            .read(app)
            .value()
            .to_string()
    });
    assert!(id.starts_with("claude-cli"), "id follows the detected kind's slug, got {id}");
}

#[test]
fn late_scan_reseeds_untouched_wizard() {
    let mut app = TestAppContext::single();
    // Nothing detected at open — the wizard starts on the Codex default.
    set_probe(fake(&[], &[]));
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| panel.open_provider_wizard(window, cx));
    });
    settle(cx);
    assert_eq!(ws.read_with(cx, |w, app| w.settings_panel.read(app).provider_wizard.as_ref().unwrap().kind), ProviderKind::CodexCli);
    // A CLI appears and a fresh scan lands — the open wizard follows it.
    set_probe(fake(&["claude"], &[]));
    cx.update(|_, cx| ws.update(cx, |w, cx| w.detect_providers(cx)));
    settle(cx);
    let (kind, detected) = ws.read_with(cx, |w, app| {
        let w = w.settings_panel.read(app).provider_wizard.as_ref().unwrap();
        (w.kind, w.detected.clone())
    });
    assert_eq!(kind, ProviderKind::ClaudeCli, "untouched wizard follows the scan");
    assert_eq!(detected, vec![ProviderKind::ClaudeCli]);
}

#[test]
fn user_pick_survives_late_scan() {
    let mut app = TestAppContext::single();
    set_probe(fake(&[], &[]));
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| panel.open_provider_wizard(window, cx));
    });
    settle(cx);
    // The user picks Sim before the scan lands.
    cx.update(|window, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| panel.reseed_wizard(ProviderKind::Sim, window, cx));
    });
    set_probe(fake(&["claude"], &[]));
    cx.update(|_, cx| ws.update(cx, |w, cx| w.detect_providers(cx)));
    settle(cx);
    let (kind, detected) = ws.read_with(cx, |w, app| {
        let w = w.settings_panel.read(app).provider_wizard.as_ref().unwrap();
        (w.kind, w.detected.clone())
    });
    assert_eq!(kind, ProviderKind::Sim, "a user pick is never overridden");
    assert_eq!(detected, vec![ProviderKind::ClaudeCli], "chips still refresh");
}

// ---- onboarding card copy ----

#[test]
fn card_button_names_detected_provider() {
    let mut app = TestAppContext::single();
    set_probe(fake(&["codex"], &[]));
    let (ws, cx) = mount(&mut app);
    // Drop every provider so the card shows, then let the render-triggered
    // scan land.
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let ids: Vec<String> = w.provider_instances().iter().map(|p| p.id.clone()).collect();
            for id in ids {
                w.remove_provider(&id, cx);
            }
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    settle(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("onboarding-card").visible());
        let button = window.find("onboarding-setup");
        assert!(button.visible());
        assert_eq!(button.label(), Some("Set up Codex CLI"));
    });
}

#[test]
fn card_button_stays_generic_without_detections() {
    let mut app = TestAppContext::single();
    set_probe(fake(&[], &[]));
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let ids: Vec<String> = w.provider_instances().iter().map(|p| p.id.clone()).collect();
            for id in ids {
                w.remove_provider(&id, cx);
            }
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    settle(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find("onboarding-setup").label(), Some("Set up a provider"));
    });
}
