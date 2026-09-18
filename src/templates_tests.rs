//! Tests for prompt templates: the `TemplateStore` ops, `templates.json`
//! persistence, the `/templates` picker (list / load / delete / empty
//! state), and the composer ⋯ menu's "Save as template…" dialog.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::TestAppContext;
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Role as A11yRole, Window};

use crate::composer_testutil::{composer_value, open_workspace, settle_dialog, type_and_send, user_msgs};
use crate::prompts::TemplateStore;

#[test]
fn store_save_get_remove() {
    let mut store = TemplateStore::default();
    assert!(store.save("review", "review this diff"));
    assert_eq!(store.get("review").map(|t| t.body.as_str()), Some("review this diff"));
    // Re-saving the same name overwrites the body without duplicating.
    assert!(store.save("review", "review the diff carefully"));
    assert_eq!(store.templates.len(), 1);
    assert_eq!(store.get("review").map(|t| t.body.as_str()), Some("review the diff carefully"));
    // Empty name or body saves nothing.
    assert!(!store.save("", "x"));
    assert!(!store.save("x", "  "));
    // Remove drops exactly the named template.
    assert!(store.remove("review"));
    assert!(!store.remove("review"));
    assert!(store.templates.is_empty());
}

#[test]
fn store_persists_roundtrip() {
    let dir = std::env::temp_dir().join(format!("rixlcode-templates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = TemplateStore::default();
    store.save("a", "alpha");
    store.save("b", "beta");
    crate::persist::save_templates(&dir, &store);
    let loaded = crate::persist::load_templates(&dir);
    assert_eq!(loaded.templates, store.templates);
    // An empty store removes the file — a cleared list stays cleared.
    crate::persist::save_templates(&dir, &TemplateStore::default());
    assert!(!dir.join("templates.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open the ⋯ menu and click the item labelled `label`.
fn click_menu_item(window: &mut Window, cx: &mut gpui_kit::App, label: &str) {
    window.draw(cx).clear(cx);
    window.click("composer-menu", cx);
    window.draw(cx).clear(cx);
    let item = snapshots(window)
        .iter()
        .find(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some(label))
        .unwrap_or_else(|| panic!("composer menu should offer {label}"))
        .clone();
    window.within("popup-menu").click(item.path().last().unwrap().clone(), cx);
    window.draw(cx).clear(cx);
}

#[gpui_kit::test]
fn save_template_dialog_saves_draft(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.input("draft to keep", cx);
        click_menu_item(window, cx, "Save as template…");
        assert!(window.try_find("dialog").is_some(), "Save as template… should open the dialog");
        let input = workspace.read(cx).template_input.clone();
        input.update(cx, |s, cx| s.set_value("draft", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.templates.get("draft").map(|t| t.body.as_str()), Some("draft to keep"));
    });
    // The save persisted — templates.json round-trips.
    let dir = workspace.read_with(cx, |ws, _| ws.project.dir().to_path_buf());
    let loaded = crate::persist::load_templates(&dir);
    assert_eq!(loaded.get("draft").map(|t| t.body.as_str()), Some("draft to keep"));
}

#[gpui_kit::test]
fn picker_lists_in_saved_order(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    workspace.update(cx, |ws, _| {
        ws.templates.save("zebra", "z body");
        ws.templates.save("apple", "a body");
        ws.templates.save("mango", "m body");
    });
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_template_picker(window, cx)));
    settle_dialog(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let top = |id: &'static str| f32::from(window.find(id).bounds().origin.y);
        assert!(top("template-row-zebra") < top("template-row-apple"), "saved order, not sorted");
        assert!(top("template-row-apple") < top("template-row-mango"), "saved order, not sorted");
    });
}

#[gpui_kit::test]
fn pick_loads_draft_without_sending(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    workspace.update(cx, |ws, _| {
        ws.templates.save("greet", "say hello warmly");
    });
    type_and_send(cx, "/templates");
    settle_dialog(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("templates-list").visible(), "/templates should open the picker");
        window.click("template-load-greet", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("templates-list").is_none(), "picking a template closes the picker");
    });
    assert_eq!(composer_value(&workspace, cx), "say hello warmly", "picking a template loads it into the composer");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(user_msgs(ws, "say hello"), 0, "loading a template must not send it");
    });
}

#[gpui_kit::test]
fn picker_delete_removes_template(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    workspace.update(cx, |ws, _| {
        ws.templates.save("greet", "say hello");
    });
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_template_picker(window, cx)));
    settle_dialog(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The ✕ is hover-revealed — hidden until the row is hovered.
        assert!(!window.find("template-delete-greet").visible(), "delete hidden before hover");
        window.hover("template-row-greet", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("template-delete-greet").visible(), "delete reveals on hover");
        window.click("template-delete-greet", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("template-row-greet").is_none(), "deleted template should leave the list");
        assert!(window.find("templates-empty").visible(), "empty list shows the empty state");
    });
    workspace.read_with(cx, |ws, _| assert!(ws.templates.get("greet").is_none()));
    let dir = workspace.read_with(cx, |ws, _| ws.project.dir().to_path_buf());
    assert!(crate::persist::load_templates(&dir).templates.is_empty(), "delete persists");
}

#[gpui_kit::test]
fn picker_empty_state(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_template_picker(window, cx)));
    settle_dialog(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("templates-empty").visible(), "empty store shows the empty state");
    });
}
