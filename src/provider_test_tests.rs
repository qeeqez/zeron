//! Headless tests for the Providers "Test connection" probe: the row
//! button goes inert while a probe runs, a landed Ok shows the model count
//! and latency, a landed Err shows the message, and the wizard's Config
//! step probes its draft instance. A submodule of
//! `settings_providers_tests` so it shares the mount/open helpers.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use super::{mi, mount, open_providers, reveal_last_row};
use crate::views::settings_provider_test::TestState;

/// Land a probe outcome on the panel the way the background task would.
fn land(
    ws: &gpui_kit::Entity<crate::workspace::Workspace>, cx: &mut gpui_kit::VisualTestContext, id: &str,
    result: Result<Vec<crate::model::ModelInfo>, String>, ms: u64,
) {
    cx.update(|_, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| panel.land_provider_test(id, result, ms, cx));
    });
}

#[test]
fn test_button_lands_ok_with_count_and_latency() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    reveal_last_row(cx);
    cx.update(|window, cx| {
        assert!(window.find("provider-test-sim").visible(), "every instance row gets a Test button");
        window.click("provider-test-sim", cx);
        window.draw(cx).clear(cx);
        // The probe spawn is stubbed in tests — the row stays Testing and
        // the button inert (spinner + relabel) until a result lands. GPUI
        // has no aria-disabled setter, so inertness shows via the label.
        assert_eq!(window.find("provider-test-sim").label(), Some("Testing…"));
        assert!(window.try_find("provider-test-result-sim").is_none(), "no result line while testing");
    });
    let state = ws.read_with(cx, |w, app| w.settings_panel.read(app).test_state.get("sim").cloned());
    assert_eq!(state, Some(TestState::Testing));

    land(&ws, cx, "sim", Ok(vec![mi("m1"), mi("m2"), mi("m3")]), 42);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let result = window.find("provider-test-result-sim");
        assert!(result.visible(), "result line shows after the probe lands");
        assert_eq!(result.label(), Some("3 models · 42ms"));
        assert_eq!(window.find("provider-test-sim").label(), Some("Test"), "button is clickable again");
    });
    let state = ws.read_with(cx, |w, app| w.settings_panel.read(app).test_state.get("sim").cloned());
    assert_eq!(state, Some(TestState::Ok { models: 3, ms: 42 }));
    // A successful probe doubles as a catalog refresh.
    let models = ws.read_with(cx, |w, _| w.models_for("sim").iter().map(|m| m.id.to_string()).collect::<Vec<_>>());
    assert_eq!(models, ["m1", "m2", "m3"]);
}

#[test]
fn test_button_lands_error_first_line() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    reveal_last_row(cx);
    cx.update(|window, cx| {
        window.click("provider-test-sim", cx);
        window.draw(cx).clear(cx);
    });
    land(&ws, cx, "sim", Err("codex spawn: not found\ninstall codex first".to_string()), 7);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let result = window.find("provider-test-result-sim");
        assert!(result.visible());
        // The accessibility label carries the full error (the tooltip's
        // text); the visible line is just the first line.
        assert_eq!(result.label(), Some("codex spawn: not found\ninstall codex first"));
    });
    let state = ws.read_with(cx, |w, app| w.settings_panel.read(app).test_state.get("sim").cloned());
    assert_eq!(state, Some(TestState::Err("codex spawn: not found\ninstall codex first".to_string())));
}

#[test]
fn testing_state_blocks_reclick() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    reveal_last_row(cx);
    cx.update(|window, cx| {
        window.click("provider-test-sim", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("provider-test-sim").label(), Some("Testing…"));
        // A second click while Testing must not restart or clear the probe.
        window.click("provider-test-sim", cx);
        window.draw(cx).clear(cx);
    });
    let state = ws.read_with(cx, |w, app| w.settings_panel.read(app).test_state.get("sim").cloned());
    assert_eq!(state, Some(TestState::Testing), "re-click must not disturb an in-flight probe");
}

#[test]
fn wizard_config_step_tests_the_draft() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    super::open_wizard(cx);
    cx.update(|window, cx| {
        window.click("wizard-kind-sim", cx);
        window.draw(cx).clear(cx);
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("wizard-config").visible(), "config step should show");
        assert!(window.find("wizard-test").visible(), "config step has a Test button");
        window.click("wizard-test", cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find("wizard-test").label(), Some("Testing…"), "wizard test button goes inert");
    });
    let state = ws.read_with(cx, |w, app| w.settings_panel.read(app).provider_wizard.as_ref().map(|w| w.test_state.clone()));
    assert_eq!(state, Some(TestState::Testing));
    // Land an error — the draft probe reports it without saving anything.
    cx.update(|_, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| panel.land_wizard_test(Err("no such command".to_string()), 3, cx));
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find("wizard-test-result").label(), Some("no such command"));
        assert_eq!(window.find("wizard-test").label(), Some("Test connection"));
    });
}
