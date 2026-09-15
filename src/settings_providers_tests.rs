//! Headless tests for the Providers settings section: the master-detail
//! layout (scrollable instance list + detail panel), the three-step
//! add-provider wizard, enable/remove switches, per-model toggles and
//! ordering, and the connection-field → `configure_provider` wiring.

mod env_tests;

use gpui_kit::component::Root;
use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, ScrollDelta, SharedString, TestAppContext, VisualTestContext, point, px};

use crate::model::ModelInfo;
use crate::providers::ProviderKind;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-prov-settings-test-{}", std::process::id()));
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

/// Open settings and switch to the Providers section.
fn open_providers(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-providers", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-providers").visible(), "providers section should show");
    });
}

/// Open the add-provider wizard and wait out its slide-down animation — the
/// dialog animates on a real-time clock, so clicks issued before it settles
/// land on the backdrop and are swallowed.
fn open_wizard(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.click("provider-add", cx);
        window.draw(cx).clear(cx);
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("wizard-driver").visible(), "driver step should open");
    });
}

/// Drive the wizard to completion for `kind`: Driver card → Next → Next →
/// Add provider. Asserts the dialog closed and the instance exists.
fn run_wizard(cx: &mut VisualTestContext, kind: ProviderKind) {
    let slug = kind.slug();
    open_wizard(cx);
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("wizard-kind-{slug}")), cx);
        window.draw(cx).clear(cx);
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("wizard-identity").visible(), "identity step should show");
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("wizard-config").visible(), "config step should show");
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("wizard-driver").is_none(), "dialog should close after finish");
    });
}

#[test]
fn wizard_adds_an_instance_per_kind() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    for kind in ProviderKind::ALL {
        run_wizard(cx, kind);
    }
    let instances = ws.read_with(cx, |w, _| w.provider_instances().to_vec());
    // The seeded set already has one instance per kind; the wizard adds a
    // second of each — ids dedup to `<slug>-2`.
    assert_eq!(instances.len(), ProviderKind::ALL.len() * 2);
    for kind in ProviderKind::ALL {
        let id = format!("{}-2", kind.slug());
        let p = instances.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(p.kind, kind);
        assert!(p.enabled);
        // Rows beyond the fold are clipped by the scroll container — assert
        // presence, not visibility.
        assert!(cx.update(|window, _| window.try_find(SharedString::from(format!("provider-row-{id}"))).is_some()));
    }
}

#[test]
fn instance_list_shows_only_real_instances_and_scrolls() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    cx.update(|window, cx| {
        // Every persisted instance renders a row; nothing else does.
        for p in ws.read(cx).provider_instances() {
            assert!(window.find(SharedString::from(format!("provider-row-{}", p.id))).visible(), "missing row for {}", p.id);
        }
        assert!(window.try_find("provider-row-ghost").is_none(), "no placeholder rows");
        // The list is a real scroll container — a wheel event lands on it.
        window.scroll("provider-list", ScrollDelta::Pixels(point(px(0.), px(120.))), cx);
        window.draw(cx).clear(cx);
        assert!(window.find("provider-list").visible());
    });
}

#[test]
fn provider_enable_and_remove_work() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    cx.update(|window, cx| {
        window.click("provider-enable-sim", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).provider_instances().iter().find(|p| p.id == "sim").unwrap().enabled);
        window.click("provider-enable-sim", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).provider_instances().iter().find(|p| p.id == "sim").unwrap().enabled);

        window.click(SharedString::from("provider-row-sim"), cx);
        window.draw(cx).clear(cx);
        window.click("provider-remove-sim", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).provider_instances().iter().all(|p| p.id != "sim"), "sim should be removed");
        assert!(window.try_find("provider-row-sim").is_none(), "row should unmount");
    });
}

#[test]
fn model_toggle_and_reorder_update_config() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        this.land_catalog("sim", vec![mi("m1"), mi("m2"), mi("m3")], cx);
    });
    open_providers(cx);
    cx.update(|window, cx| {
        window.click("provider-row-sim", cx);
        window.draw(cx).clear(cx);
        window.click("model-toggle-sim-m1", cx);
        window.draw(cx).clear(cx);
        let cfg = ws.read(cx).models_config_for("sim");
        assert_eq!(cfg.iter().find(|(m, _)| m.id == "m1").map(|(_, on)| *on), Some(false));

        window.click("model-up-sim-m2", cx);
        window.draw(cx).clear(cx);
        let order: Vec<String> = ws.read(cx).models_config_for("sim").iter().map(|(m, _)| m.id.to_string()).collect();
        assert_eq!(order, ["m2", "m1", "m3"], "m2 should move ahead of m1");
    });
}

#[test]
fn connection_fields_write_through_configure_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    cx.update(|window, cx| {
        window.click("provider-row-http", cx);
        window.draw(cx).clear(cx);
        let inputs = ws.read(cx).settings_panel.read(cx).provider_inputs["http"].clone();
        inputs.command.update(cx, |s, cx| {
            s.set_value("https://api.example.com", window, cx);
            cx.emit(InputEvent::Change);
        });
        inputs.key_env.update(cx, |s, cx| {
            s.set_value("MY_KEY", window, cx);
            cx.emit(InputEvent::Change);
        });
    });
    // `emit` queues an effect — it dispatches when the update returns.
    cx.update(|window, cx| {
        let http = ws.read(cx).provider_instances().iter().find(|p| p.id == "http").unwrap().clone();
        assert_eq!((http.command.as_str(), http.key_env.as_str()), ("https://api.example.com", "MY_KEY"));

        // The display-name field renames the instance.
        let inputs = ws.read(cx).settings_panel.read(cx).provider_inputs["http"].clone();
        inputs.name.update(cx, |s, cx| {
            s.set_value("Work HTTP", window, cx);
            cx.emit(InputEvent::Change);
        });
    });
    cx.update(|_window, cx| {
        assert_eq!(ws.read(cx).provider_instances().iter().find(|p| p.id == "http").unwrap().name, "Work HTTP");
    });
}

#[test]
fn wizard_rejects_duplicate_id() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_providers(cx);
    open_wizard(cx);
    cx.update(|window, cx| {
        window.click("wizard-kind-sim", cx);
        window.draw(cx).clear(cx);
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        // Force a collision with the existing "sim" instance.
        let id_input = ws.read(cx).settings_panel.read(cx).provider_wizard.as_ref().unwrap().instance_id.clone();
        id_input.update(cx, |s, cx| s.set_value("sim", window, cx));
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        window.click("wizard-next", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("wizard-error").visible(), "duplicate id should show an error");
        assert!(window.find("wizard-config").visible(), "dialog stays open on the config step");
        assert_eq!(ws.read(cx).provider_instances().iter().filter(|p| p.id == "sim").count(), 1);
    });
}

fn mi(id: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        label: id.into(),
        description: Default::default(),
        ..Default::default()
    }
}

#[test]
fn provider_row_shows_auth_status_and_sign_in() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Land a signed-out state on the seeded codex instance.
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            w.land_auth("codex-cli", crate::auth::AuthState::SignedOut, cx);
        });
    });
    open_providers(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("provider-sign-in-codex-cli").visible(), "signed-out row shows a sign-in button");
        // The detail panel shows the status line and the same button.
        assert!(window.find("provider-detail").visible());
    });
    // Click it — the flow starts, the button is replaced by Cancel.
    cx.update(|window, cx| {
        window.click("provider-sign-in-codex-cli", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("provider-sign-in-codex-cli").is_none(), "sign-in hides while the flow runs");
        assert!(window.find("auth-cancel-codex-cli").visible(), "cancel shows while signing in");
    });
    let state = ws.read_with(cx, |w, _| w.auth_state("codex-cli"));
    assert!(matches!(state, crate::auth::AuthState::SigningIn(_)));
    // Cancel restores the signed-out row.
    cx.update(|window, cx| {
        window.click("auth-cancel-codex-cli", cx);
        window.draw(cx).clear(cx);
    });
    let state = ws.read_with(cx, |w, _| w.auth_state("codex-cli"));
    assert_eq!(state, crate::auth::AuthState::Unknown);
}
