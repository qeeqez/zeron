//! Provider-instance tests: settings migration from the legacy flat fields,
//! `models_for` filtering/ordering, selection, add/remove/enable, model
//! enable/reorder, and the no-`default` send path.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, SharedString, TestAppContext, VisualTestContext};

use crate::model::ModelInfo;
use crate::persist::Settings;
use crate::providers::{ModelConfig, ProviderKind, apply_model_config};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-providers-test-{}", std::process::id()));
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

fn mi(id: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        label: id.into(),
        description: SharedString::default(),
    }
}

fn cfg(id: &str, enabled: bool, order: u32) -> ModelConfig {
    ModelConfig { id: id.into(), enabled, order }
}

// ---- settings migration ----

#[test]
fn legacy_flat_settings_migrate_to_instances() {
    let mut s: Settings = serde_json::from_str(
        r#"{"backend":"claude-cli","model":"sonnet","disabled_providers":["sim"],"http_url":"https://x","http_key_env":"K","acp_command":"agent --flag"}"#,
    )
    .unwrap();
    s.migrate_legacy();
    assert_eq!(s.providers.len(), ProviderKind::ALL.len(), "one instance per known kind");
    assert_eq!(s.selected_provider, "claude-cli");
    assert_eq!(s.selected_model, "sonnet");
    let http = s.providers.iter().find(|p| p.kind == ProviderKind::Http).unwrap();
    assert_eq!((http.command.as_str(), http.key_env.as_str()), ("https://x", "K"));
    let acp = s.providers.iter().find(|p| p.kind == ProviderKind::Acp).unwrap();
    assert_eq!(acp.command, "agent --flag");
    assert!(!s.providers.iter().find(|p| p.id == "sim").unwrap().enabled, "disabled_providers → enabled flag");
    // Legacy fields are consumed — a second pass is a no-op.
    s.migrate_legacy();
    assert_eq!(s.providers.len(), ProviderKind::ALL.len());
}

#[test]
fn legacy_default_model_migrates_to_first() {
    let mut s: Settings = serde_json::from_str(r#"{"backend":"codex-cli","model":"default"}"#).unwrap();
    s.migrate_legacy();
    assert_eq!(s.selected_provider, "codex-cli");
    assert_eq!(s.selected_model, "", "the synthetic default is gone — empty resolves to the first model");
}

#[test]
fn legacy_bool_selects_backend() {
    let mut s: Settings = serde_json::from_str(r#"{"use_codex_cli":false}"#).unwrap();
    s.migrate_legacy();
    assert_eq!(s.selected_provider, "sim");
}

#[test]
fn legacy_disabled_selected_provider_falls_forward() {
    let mut s: Settings = serde_json::from_str(r#"{"backend":"codex-cli","disabled_providers":["codex-cli"]}"#).unwrap();
    s.migrate_legacy();
    assert_eq!(s.selected_provider, "claude-cli", "disabled selection moves to the first enabled instance");
}

#[test]
fn new_format_file_is_not_migrated() {
    let json = serde_json::to_string(&Settings {
        providers: vec![crate::providers::ProviderInstance::new(ProviderKind::Sim, "Sim".into())],
        selected_provider: "sim".into(),
        selected_model: "sim".into(),
        ..Default::default()
    })
    .unwrap();
    let mut s: Settings = serde_json::from_str(&json).unwrap();
    s.migrate_legacy();
    assert_eq!(s.providers.len(), 1);
    assert_eq!(s.selected_provider, "sim");
    // Legacy fields never serialize.
    assert!(!json.contains("\"backend\"") && !json.contains("\"disabled_providers\""));
}

// ---- models_for filtering/ordering ----

#[test]
fn apply_model_config_orders_and_filters() {
    let catalog = vec![mi("a"), mi("b"), mi("c")];
    // Empty config → catalog order, all enabled.
    assert_eq!(apply_model_config(&catalog, &[]).iter().map(|m| m.id.as_ref()).collect::<Vec<_>>(), ["a", "b", "c"]);
    // Configured models lead in `order`; disabled drop; unconfigured keep
    // catalog order after them.
    let config = vec![cfg("c", true, 0), cfg("a", false, 1), cfg("b", true, 2)];
    assert_eq!(apply_model_config(&catalog, &config).iter().map(|m| m.id.as_ref()).collect::<Vec<_>>(), ["c", "b"]);
}

// ---- workspace API ----

#[test]
fn select_model_switches_backend_and_model() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let changed = ws.update(cx, |this, cx| this.select_model("claude-cli", "opus", cx));
    assert!(changed);
    let (provider, model, backend) =
        ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.model.to_string(), w.backend.name()));
    assert_eq!((provider.as_deref(), model.as_str(), backend), (Some("claude-cli"), "opus", "claude-cli"));
    // Unknown model id → refused.
    assert!(!ws.update(cx, |this, cx| this.select_model("claude-cli", "nope", cx)));
    // Unknown instance → refused.
    assert!(!ws.update(cx, |this, cx| this.select_model("nope", "opus", cx)));
}

#[test]
fn add_remove_provider_roundtrip() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let id = ws.update(cx, |this, cx| this.add_provider(ProviderKind::Http, "Work HTTP".into(), cx));
    assert_eq!(id, "http-2", "second http instance gets a deduped id");
    let names: Vec<String> = ws.read_with(cx, |w, _| w.provider_instances().iter().map(|p| p.name.clone()).collect());
    assert!(names.contains(&"Work HTTP".to_string()));
    ws.update(cx, |this, cx| this.remove_provider(&id, cx));
    assert!(!ws.read_with(cx, |w, _| w.provider_instances().iter().any(|p| p.id == id)));
}

#[test]
fn removing_selected_provider_moves_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    assert_eq!(ws.read_with(cx, |w, _| w.selected_provider().map(str::to_string)), Some("codex-cli".to_string()));
    ws.update(cx, |this, cx| this.remove_provider("codex-cli", cx));
    let (provider, backend) = ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.backend.name()));
    assert_eq!((provider.as_deref(), backend), (Some("claude-cli"), "claude-cli"));
}

#[test]
fn set_model_enabled_and_move_model() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| {
        this.land_catalog("codex-cli", vec![mi("a"), mi("b"), mi("c")], cx);
        this.set_model_enabled("codex-cli", "a", false, cx);
    });
    let ids: Vec<String> = ws.read_with(cx, |w, _| w.models_for("codex-cli").iter().map(|m| m.id.to_string()).collect());
    assert_eq!(ids, ["b", "c"], "disabled model drops out of the effective list");
    ws.update(cx, |this, cx| this.move_model("codex-cli", "c", -1, cx));
    let ids: Vec<String> = ws.read_with(cx, |w, _| w.models_for("codex-cli").iter().map(|m| m.id.to_string()).collect());
    assert_eq!(ids, ["c", "b"], "move_model reorders the effective list");
    // Disabling the selected model re-resolves the selection.
    ws.update(cx, |this, cx| {
        assert!(this.select_model("codex-cli", "c", cx));
        this.set_model_enabled("codex-cli", "c", false, cx);
    });
    assert_eq!(ws.read_with(cx, |w, _| w.model.to_string()), "b");
}

#[test]
fn send_with_no_model_is_an_error_not_a_default() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // acp has no catalog until its first session — sending there can't
    // invent a "default" model.
    ws.update(cx, |this, cx| this.set_provider_enabled("acp", true, cx));
    ws.update(cx, |this, _cx| {
        this.selected_provider = "acp".into();
        this.model = "".into();
        this.backend = crate::backend::backend_for(this.providers.iter().find(|p| p.id == "acp").unwrap());
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    let (running, last) = ws.read_with(cx, |w, _| {
        let chat = &w.chats[w.active];
        let text = chat.messages.last().and_then(|m| match &m.kind {
            crate::model::MessageKind::Text(t) => Some(t.to_string()),
            _ => None,
        });
        (chat.running, text)
    });
    assert!(!running, "the turn must not hang");
    assert!(last.as_ref().is_some_and(|t| t.contains("no models")), "expected an error note, got {last:?}");
}
