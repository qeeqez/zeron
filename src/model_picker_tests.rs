//! Headless tests for the two-pane provider→model picker: the left pane
//! lists enabled instances with their kind icons, the right pane lists the
//! browsed instance's models, picks switch instance+model and close the
//! popover, and disabled instances stay out. There is no synthetic
//! `default` entry — a provider with no catalog shows an empty state.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, SharedString, TestAppContext, VisualTestContext, Window};

use crate::model::ModelInfo;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-models-test-{}", std::process::id()));
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

fn mi(id: &str, label: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        label: label.into(),
        description: SharedString::default(),
    }
}

/// Inject a fetched catalog the way `refresh_model_catalogs` would.
fn inject(ws: &Entity<Workspace>, cx: &mut VisualTestContext, instance: &str, models: Vec<ModelInfo>) {
    ws.update(cx, |this, cx| this.land_catalog(instance, models, cx));
}

/// Open the picker popover and let the deferred content paint.
fn open_picker(window: &mut Window, cx: &mut App) {
    window.draw(cx).clear(cx);
    window.click("model", cx);
    window.draw(cx).clear(cx);
    window.draw(cx).clear(cx);
}

#[test]
fn picker_options_have_no_default_entry() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9"), mi("gpt-8", "GPT-8")]);
    let ids: Vec<String> = ws.read_with(cx, |w, _| w.models_for("codex-cli").iter().map(|m| m.id.to_string()).collect());
    assert_eq!(ids, ["gpt-9", "gpt-8"]);
}

#[test]
fn picker_opens_two_panes_listing_providers_and_models() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9")]);
    cx.update(|window, cx| {
        open_picker(window, cx);
        assert!(window.find("model-picker-panes").visible(), "picker popover should open");
        // LEFT: every enabled instance renders a row.
        for id in ["codex-cli", "claude-cli", "acp", "http", "sim"] {
            assert!(window.find(format!("provider-{id}")).visible(), "provider {id} should be listed");
        }
        // RIGHT: the active provider's models — injected catalog, no default row.
        assert!(window.try_find("model-opt-codex-cli-default").is_none(), "no synthetic default row");
        assert!(window.find("model-opt-codex-cli-gpt-9").visible(), "injected model should render");
        // Another provider's models stay out of the right pane.
        assert!(window.try_find("model-opt-sim-sim").is_none(), "only the browsed provider's models render");
    });
}

#[test]
fn browsing_provider_swaps_the_model_pane() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9")]);
    cx.update(|window, cx| {
        open_picker(window, cx);
        window.click("provider-sim", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("model-opt-sim-sim").visible(), "browsed provider's model should render");
        assert!(window.try_find("model-opt-codex-cli-gpt-9").is_none(), "previous provider's models are gone");
    });
    // Browsing must not change the selection.
    let provider = ws.read_with(cx, |w, _| w.selected_provider().map(str::to_string));
    assert_eq!(provider.as_deref(), Some("codex-cli"));
}

#[test]
fn picking_model_selects_provider_and_model() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "sim", vec![mi("sim-x", "Sim X")]);
    cx.update(|window, cx| {
        open_picker(window, cx);
        window.click("provider-sim", cx);
        window.draw(cx).clear(cx);
        window.click("model-opt-sim-sim-x", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("model-picker-panes").is_none(), "picking a model closes the popover");
    });
    let (provider, model, backend) =
        ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.model.to_string(), w.backend.name()));
    assert_eq!((provider.as_deref(), model.as_str(), backend), (Some("sim"), "sim-x", "sim"));
}

#[test]
fn disabled_provider_hidden_and_unselectable() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "sim", vec![mi("sim-x", "Sim X")]);
    ws.update(cx, |this, cx| this.set_provider_enabled("sim", false, cx));
    assert!(!ws.read_with(cx, |w, _| w.enabled_providers().iter().any(|p| p.id == "sim")));
    cx.update(|window, cx| {
        open_picker(window, cx);
        assert!(window.try_find("provider-sim").is_none(), "disabled provider must not render");
        assert!(window.find("provider-http").visible(), "enabled providers still render");
    });
    // And the command path refuses it too.
    let changed = ws.update(cx, |this, cx| this.select_model("sim", "sim-x", cx));
    assert!(!changed, "disabled provider must not be selectable");
}

#[test]
fn empty_provider_shows_empty_state() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // acp learns its catalog at runtime — nothing injected, nothing listed.
    assert!(ws.read_with(cx, |w, _| w.models_for("acp").is_empty()));
    cx.update(|window, cx| {
        open_picker(window, cx);
        window.click("provider-acp", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("model-picker-empty").visible(), "empty provider shows an empty state");
        assert!(window.try_find("model-opt-acp-default").is_none(), "no synthetic default row");
    });
}

#[test]
fn disabling_active_provider_moves_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert_eq!(ws.read_with(cx, |w, _| w.selected_provider().map(str::to_string)), Some("codex-cli".to_string()));
    ws.update(cx, |this, cx| this.set_provider_enabled("codex-cli", false, cx));
    let (provider, model) = ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.model.to_string()));
    assert_eq!(provider.as_deref(), Some("claude-cli"), "selection moves to the first enabled provider");
    assert_eq!(model, "sonnet", "model resolves to the new provider's first model");
}

#[test]
fn disabling_last_provider_clears_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        for id in ["codex-cli", "claude-cli", "acp", "http", "sim"] {
            this.set_provider_enabled(id, false, cx);
        }
    });
    let (provider, model) = ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.model.to_string()));
    assert_eq!((provider, model.as_str()), (None, ""), "no enabled provider → empty selection");
}

#[test]
fn land_catalog_resets_removed_model() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9")]);
    ws.update(cx, |this, cx| {
        assert!(this.select_model("codex-cli", "gpt-9", cx));
    });
    // A refresh that drops gpt-9 resets the selection to the first model.
    inject(&ws, cx, "codex-cli", vec![mi("gpt-10", "GPT-10")]);
    assert_eq!(ws.read_with(cx, |w, _| w.model.to_string()), "gpt-10");
}

#[test]
fn empty_fetch_keeps_existing_catalog() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let before = ws.read_with(cx, |w, _| w.models_for("codex-cli").len());
    inject(&ws, cx, "codex-cli", Vec::new());
    let after = ws.read_with(cx, |w, _| w.models_for("codex-cli").len());
    assert_eq!(before, after, "an empty fetch must not wipe the catalog");
    assert!(after > 0, "codex seeds from its static fallback");
}
