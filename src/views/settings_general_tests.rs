//! Headless tests for the General settings section's thread-default
//! controls: the default-model two-pane picker (writes `default_model`,
//! never the active selection), the permissions select over all four
//! `AccessMode`s, the workspace select over both `WorkspaceMode`s, and
//! persistence of each across a save/load.

use gpui_kit::component::Root;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, SharedString, TestAppContext, VisualTestContext, Window};

use super::settings_general::workspace_mode_label;
use crate::backend::AccessMode;
use crate::model::ModelInfo;
use crate::workspace::Workspace;
use crate::worktree::WorkspaceMode;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-general-test-{}", std::process::id()));
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
        ..Default::default()
    }
}

/// Open settings on the General section (the default) and paint it.
fn open_general(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.open_settings(window, cx));
    window.draw(cx).clear(cx);
}

/// Open the default-model popover and let the deferred content paint.
fn open_default_picker(window: &mut Window, cx: &mut App) {
    window.click("default-model", cx);
    window.draw(cx).clear(cx);
    window.draw(cx).clear(cx);
}

#[test]
fn general_section_shows_thread_default_controls() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        open_general(&ws, window, cx);
        assert!(window.find("settings-section-general").visible());
        for (id, caption) in [
            ("caption-model", "Default model for new threads. Projects can override it."),
            ("caption-permissions", "Default permissions for new threads."),
            ("caption-workspace", "Where new threads start."),
        ] {
            let el = window.find(id);
            assert!(el.visible(), "{id} should render");
            assert_eq!(el.label(), Some(caption), "{id} caption");
        }
        for id in ["default-model", "default-permissions", "default-workspace"] {
            assert!(window.find(id).visible(), "{id} should render");
        }
    });
}

#[test]
fn default_model_picker_sets_default_not_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let before = ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.model.to_string()));
    ws.update(cx, |this, cx| this.land_catalog("sim", vec![mi("sim-x", "Sim X")], cx));
    cx.update(|window, cx| {
        open_general(&ws, window, cx);
        open_default_picker(window, cx);
        assert!(window.find("default-model-picker-panes").visible(), "picker popover should open");
        for id in ["codex-cli", "claude-cli", "acp", "http", "sim"] {
            assert!(window.find(format!("default-provider-{id}")).visible(), "provider {id} should be listed");
        }
        window.click("default-provider-sim", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("default-model-opt-sim-sim-x").visible(), "browsed provider's model should render");
        window.click("default-model-opt-sim-sim-x", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("default-model-picker-panes").is_none(), "picking a model closes the popover");
    });
    let dm = ws.read_with(cx, |w, _| w.default_model().clone());
    assert_eq!((dm.provider_instance_id.as_str(), dm.model_id.as_str()), ("sim", "sim-x"));
    // The active thread's selection must be untouched.
    let after = ws.read_with(cx, |w, _| (w.selected_provider().map(str::to_string), w.model.to_string()));
    assert_eq!(after, before, "default pick must not change the active selection");
    // And the default persisted.
    let s = crate::persist::load_settings();
    assert_eq!((s.default_model.provider_instance_id.as_str(), s.default_model.model_id.as_str()), ("sim", "sim-x"));
}

#[test]
fn permissions_select_sets_default_permissions() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let select = ws.read_with(cx, |w, cx| w.settings_panel.read(cx).permissions_select.clone());
    for mode in AccessMode::ALL {
        cx.update(|_window, cx| {
            select.update(cx, |_, cx| cx.emit(SelectEvent::<Vec<String>>::Confirm(Some(mode.label().to_string()))));
        });
        assert_eq!(ws.read_with(cx, |w, _| w.default_permissions()), Some(mode), "label {}", mode.label());
    }
    // Clearing the select returns to "follow current".
    cx.update(|_window, cx| {
        select.update(cx, |_, cx| cx.emit(SelectEvent::<Vec<String>>::Confirm(None)));
    });
    assert_eq!(ws.read_with(cx, |w, _| w.default_permissions()), None);
    assert!(crate::persist::load_settings().default_permissions.is_empty(), "cleared default must persist empty");
}

#[test]
fn workspace_select_sets_default_workspace() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let select = ws.read_with(cx, |w, cx| w.settings_panel.read(cx).workspace_select.clone());
    for mode in WorkspaceMode::ALL {
        cx.update(|_window, cx| {
            select.update(cx, |_, cx| cx.emit(SelectEvent::<Vec<String>>::Confirm(Some(workspace_mode_label(mode).to_string()))));
        });
        assert_eq!(ws.read_with(cx, |w, _| w.default_workspace()), mode, "label {}", workspace_mode_label(mode));
    }
    assert_eq!(crate::persist::load_settings().default_workspace, "worktree", "last pick must persist");
}
