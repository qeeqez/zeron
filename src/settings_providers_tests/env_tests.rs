//! Headless tests for the provider detail panel's Variables section:
//! row add/remove buttons and the key/value inputs writing through to
//! `ProviderInstance.env`. A submodule of `settings_providers_tests` so it
//! shares the mount/open helpers.

use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt;

use super::{mount, open_providers};

#[test]
fn variables_section_adds_edits_and_removes_rows() {
    let mut app = gpui_kit::TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    // The detail panel opens on the selected instance (codex-cli).
    cx.update(|window, cx| {
        assert!(window.find("provider-env-add-codex-cli").visible(), "Variables section renders");
        window.click("provider-env-add-codex-cli", cx);
        window.draw(cx).clear(cx);
        window.click("provider-env-add-codex-cli", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("provider-env-row-codex-cli-0").visible());
        assert!(window.find("provider-env-row-codex-cli-1").visible());
    });
    // Blank rows sit in memory until named — fill row 0 via its inputs.
    cx.update(|window, cx| {
        let rows = ws.read(cx).settings_panel.read(cx).provider_env_inputs["codex-cli"].clone();
        assert_eq!(rows.len(), 2);
        rows[0].key.update(cx, |s, cx| {
            s.set_value("CODEX_HOME", window, cx);
            cx.emit(InputEvent::Change);
        });
        rows[0].value.update(cx, |s, cx| {
            s.set_value("/tmp/codex", window, cx);
            cx.emit(InputEvent::Change);
        });
    });
    cx.update(|window, cx| {
        let p = ws.read(cx).provider_instances().iter().find(|p| p.id == "codex-cli").unwrap();
        assert_eq!(p.env[0], ("CODEX_HOME".to_string(), "/tmp/codex".to_string()));
        assert_eq!(p.env[1], (String::new(), String::new()), "row 1 stays blank");
        // Removing row 0 shifts the blank row to index 0.
        window.click("provider-env-rm-codex-cli-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("provider-env-row-codex-cli-0").visible());
        assert!(window.try_find("provider-env-row-codex-cli-1").is_none());
        let p = ws.read(cx).provider_instances().iter().find(|p| p.id == "codex-cli").unwrap();
        assert_eq!(p.env, vec![(String::new(), String::new())]);
    });
}
