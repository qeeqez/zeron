//! Headless tests for the provider→model picker: injected catalogs render
//! under provider submenus, `default` leads every provider's list, picks
//! switch provider+model, and disabled providers stay out of the menu.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, SharedString, TestAppContext, VisualTestContext};

use crate::model::ModelInfo;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-models-test-{}", std::process::id()));
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
fn inject(ws: &Entity<Workspace>, cx: &mut VisualTestContext, provider: &'static str, models: Vec<ModelInfo>) {
    ws.update(cx, |this, cx| this.land_catalog(provider, models, cx));
}

#[test]
fn picker_options_put_default_first() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9"), mi("gpt-8", "GPT-8")]);
    let ids: Vec<String> = ws.read_with(cx, |w, _| w.picker_options("codex-cli").iter().map(|m| m.id.to_string()).collect());
    assert_eq!(ids, ["default", "gpt-9", "gpt-8"]);
}

#[test]
fn picker_shows_injected_catalog_under_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9")]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("model", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "model menu should open");
        // Hover the first provider submenu (codex-cli) to reveal its models.
        window.within("popup-menu").hover(0usize, cx);
        window.draw(cx).clear(cx);
        assert!(window.find("model-opt-codex-cli-default").visible(), "default leads the list");
        assert!(window.find("model-opt-codex-cli-gpt-9").visible(), "injected model should render");
    });
}

#[test]
fn picking_model_selects_provider_and_model() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "sim", vec![mi("sim-x", "Sim X")]);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("model", cx);
        window.draw(cx).clear(cx);
        // sim is the last provider submenu.
        window.within("popup-menu").hover(4usize, cx);
        window.draw(cx).clear(cx);
        window.click("model-opt-sim-sim-x", cx);
        window.draw(cx).clear(cx);
    });
    let (provider, model, backend) = ws.read_with(cx, |w, _| (w.provider, w.model.to_string(), w.backend.provider_id()));
    assert_eq!((provider, model.as_str(), backend), ("sim", "sim-x", "sim"));
}

#[test]
fn disabled_provider_hidden_and_unselectable() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "sim", vec![mi("sim-x", "Sim X")]);
    ws.update(cx, |this, cx| this.set_provider_enabled("sim", false, cx));
    assert!(!ws.read_with(cx, |w, _| w.enabled_providers().iter().any(|p| p.id == "sim")));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("model", cx);
        window.draw(cx).clear(cx);
        // Four enabled providers remain; sim's submenu is gone.
        assert!(window.within("popup-menu").try_find(4usize).is_none(), "disabled provider must not render");
        window.within("popup-menu").hover(0usize, cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("model-opt-sim-sim-x").is_none(), "disabled provider's models stay hidden");
    });
    // And the command path refuses it too.
    let changed = ws.update(cx, |this, cx| this.select_model("sim", "sim-x", cx));
    assert!(!changed, "disabled provider must not be selectable");
}

#[test]
fn disabling_active_provider_moves_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert_eq!(ws.read_with(cx, |w, _| w.provider), "codex-cli");
    ws.update(cx, |this, cx| this.set_provider_enabled("codex-cli", false, cx));
    let (provider, model) = ws.read_with(cx, |w, _| (w.provider, w.model.to_string()));
    assert_eq!(provider, "claude-cli", "selection moves to the first enabled provider");
    assert_eq!(model, "default");
}

#[test]
fn last_enabled_provider_cannot_be_disabled() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        for id in ["claude-cli", "acp", "http", "sim"] {
            this.set_provider_enabled(id, false, cx);
        }
        // Only codex-cli remains — disabling it must be refused.
        this.set_provider_enabled("codex-cli", false, cx);
    });
    let (provider, enabled) = ws.read_with(cx, |w, _| (w.provider, w.enabled_providers().len()));
    assert_eq!((provider, enabled), ("codex-cli", 1));
}

#[test]
fn land_catalog_resets_removed_model() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    inject(&ws, cx, "codex-cli", vec![mi("gpt-9", "GPT-9")]);
    ws.update(cx, |this, cx| {
        assert!(this.select_model("codex-cli", "gpt-9", cx));
    });
    // A refresh that drops gpt-9 resets the selection to default.
    inject(&ws, cx, "codex-cli", vec![mi("gpt-10", "GPT-10")]);
    assert_eq!(ws.read_with(cx, |w, _| w.model.to_string()), "default");
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
