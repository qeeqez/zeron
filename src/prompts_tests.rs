//! Tests for saved prompts: the `PromptStore` ops, `prompts.json`
//! persistence, the `/save` + `/prompts` commands, and the composer's ★
//! popover (load / rename / delete / save-current).
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::TestAppContext;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt;

use crate::composer_testutil::{composer_value, open_workspace, type_and_send, until, use_sim};
use crate::model::MessageKind;
use crate::prompts::PromptStore;

#[test]
fn store_save_get_rename_remove() {
    let mut store = PromptStore::default();
    assert!(store.save("review", "review this diff"));
    assert_eq!(store.get("review").map(|p| p.text.as_str()), Some("review this diff"));
    // Re-saving the same name overwrites the text.
    assert!(store.save("review", "review the diff carefully"));
    assert_eq!(store.prompts.len(), 1);
    assert_eq!(store.get("review").map(|p| p.text.as_str()), Some("review the diff carefully"));
    // Empty name or text saves nothing.
    assert!(!store.save("", "x"));
    assert!(!store.save("x", "  "));
    // Rename works; collisions and empty names are refused.
    assert!(store.rename("review", "rev"));
    assert!(store.get("review").is_none());
    assert_eq!(store.get("rev").map(|p| p.text.as_str()), Some("review the diff carefully"));
    assert!(store.save("other", "y"));
    assert!(!store.rename("rev", "other"), "rename must not clobber an existing prompt");
    assert!(!store.rename("rev", ""));
    // Remove drops exactly the named prompt.
    assert!(store.remove("rev"));
    assert!(!store.remove("rev"));
    assert_eq!(store.prompts.len(), 1);
}

#[test]
fn store_persists_roundtrip() {
    let dir = std::env::temp_dir().join(format!("rixlcode-prompts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = PromptStore::default();
    store.save("a", "alpha");
    store.save("b", "beta");
    crate::persist::save_prompts(&dir, &store);
    let loaded = crate::persist::load_prompts(&dir);
    assert_eq!(loaded.prompts, store.prompts);
    // An empty store removes the file — a cleared list stays cleared.
    crate::persist::save_prompts(&dir, &PromptStore::default());
    assert!(!dir.join("prompts.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn save_command_stores_inline_text(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    type_and_send(cx, "/save greet say hello warmly");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.prompts.get("greet").map(|p| p.text.as_str()), Some("say hello warmly"));
        // The command consumed the input — nothing reaches the backend.
        assert!(
            ws.chats[ws.active]
                .messages
                .iter()
                .all(|m| { !matches!(&m.kind, MessageKind::Text(t) if t.contains("say hello") && m.role == crate::model::Role::User) })
        );
    });
}

#[gpui_kit::test]
fn save_command_without_text_uses_last_message(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    use_sim(&workspace, cx);
    type_and_send(cx, "remember these words");
    until(&workspace, cx, |ws| !ws.chats[ws.active].running);
    type_and_send(cx, "/save words");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.prompts.get("words").map(|p| p.text.as_str()), Some("remember these words"));
    });
}

#[gpui_kit::test]
fn prompts_command_lists_saved(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    type_and_send(cx, "/save greet say hello");
    type_and_send(cx, "/prompts");
    workspace.read_with(cx, |ws, _| {
        let listed = ws.chats[ws.active]
            .messages
            .iter()
            .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.contains("Saved prompts") && t.contains("greet")));
        assert!(listed, "/prompts should note the saved prompt");
    });
}

#[gpui_kit::test]
fn popover_lists_and_loads_prompt(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    type_and_send(cx, "/save greet say hello warmly");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("prompts", cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        assert!(window.find("prompts-list").visible(), "popover should open");
        assert!(window.find("prompt-row-greet").visible(), "saved prompt should be listed");
        window.click("prompt-load-greet", cx);
        window.draw(cx).clear(cx);
    });
    assert_eq!(composer_value(&workspace, cx), "say hello warmly", "picking a prompt loads it into the composer");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("prompts-list").is_none(), "picking a prompt closes the popover");
    });
}

#[gpui_kit::test]
fn popover_delete_removes_prompt(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    type_and_send(cx, "/save greet say hello");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("prompts", cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        window.click("prompt-delete-greet", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("prompt-row-greet").is_none(), "deleted prompt should leave the list");
        assert!(window.find("prompts-empty").visible(), "empty list shows the empty state");
    });
    assert!(workspace.read_with(cx, |ws, _| ws.prompts.get("greet").is_none()));
}

#[gpui_kit::test]
fn popover_rename_dialog_renames(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    type_and_send(cx, "/save greet say hello");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("prompts", cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        window.click("prompt-rename-greet", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_some(), "rename should open the dialog");
        let input = workspace.read(cx).prompt_input.clone();
        input.update(cx, |s, cx| s.set_value("greeting", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert!(ws.prompts.get("greet").is_none());
        assert_eq!(ws.prompts.get("greeting").map(|p| p.text.as_str()), Some("say hello"));
    });
}

#[gpui_kit::test]
fn save_current_dialog_saves_composer_text(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    cx.update(|window, cx| {
        window.input("draft to keep", cx);
        window.draw(cx).clear(cx);
        window.click("prompts", cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        window.click("prompt-save-current", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("dialog").is_some(), "Save current… should open the dialog");
        let input = workspace.read(cx).prompt_input.clone();
        input.update(cx, |s, cx| s.set_value("draft", window, cx));
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.prompts.get("draft").map(|p| p.text.as_str()), Some("draft to keep"));
    });
}

#[gpui_kit::test]
fn prompts_survive_reload(cx: &mut TestAppContext) {
    let (workspace, cx) = open_workspace(cx);
    type_and_send(cx, "/save greet say hello");
    let dir = workspace.read_with(cx, |ws, _| ws.project.dir().to_path_buf());
    let loaded = crate::persist::load_prompts(&dir);
    assert_eq!(loaded.get("greet").map(|p| p.text.as_str()), Some("say hello"), "prompts.json should round-trip");
}
