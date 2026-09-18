//! Headless tests for "Open in New Window": the spawned window binds the
//! same project, selects the chat it was opened for (keyed by the
//! persisted `created_at`, not the per-load runtime id), and never
//! duplicates the chat — plus the ⋯/context-menu wiring.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-chatwin-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
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

/// The `Workspace` entities of every open window except `known`.
fn other_workspaces(app: &mut TestAppContext, known: &Entity<Workspace>) -> Vec<Entity<Workspace>> {
    let windows = app.read(|cx| cx.windows());
    let mut found = Vec::new();
    for handle in windows {
        let _ = handle.update(app, |root, _window, cx| {
            if let Some(ws) = root.downcast::<Root>().ok().and_then(|r| r.read(cx).view().clone().downcast::<Workspace>().ok())
                && ws != *known
            {
                found.push(ws);
            }
        });
    }
    found
}

/// "Open in New Window" spawns a second workspace on the same project with
/// the chat selected — the same chat, not a copy.
#[test]
fn open_in_new_window_selects_same_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (id, created_at) = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            let chat = &this.chats[1];
            (chat.id, chat.created_at)
        })
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.open_chat_in_new_window(id, cx)));
    app.run_until_parked();

    let others = other_workspaces(&mut app, &ws);
    assert_eq!(others.len(), 1, "exactly one new window should open");
    app.read(|cx| {
        let other = others[0].read(cx);
        assert_eq!(other.project.root(), ws.read(cx).project.root(), "new window binds the same project");
        assert_eq!(other.chats.len(), 2, "the chat is shared, not duplicated");
        assert_eq!(other.chats[other.active].created_at, created_at, "the opened chat is selected");
        assert_eq!(other.chats[other.active].title, ws.read(cx).chats[1].title);
    });
}

/// Chat ids are reassigned in file order on load, so after a deletion the
/// same chat wears a different id in the new window — selection must key
/// on the persisted `created_at`, not the runtime id.
#[test]
fn open_in_new_window_survives_id_shift() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (id, created_at) = cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            this.new_chat(cx);
            let key = (this.chats[2].id, this.chats[2].created_at);
            this.delete_chat_now(0, window, cx);
            key
        })
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.open_chat_in_new_window(id, cx)));
    app.run_until_parked();

    let others = other_workspaces(&mut app, &ws);
    assert_eq!(others.len(), 1);
    app.read(|cx| {
        let other = others[0].read(cx);
        assert_eq!(other.chats.len(), 2);
        assert_ne!(other.chats[other.active].id, id, "the reloaded chat wears a different runtime id");
        assert_eq!(other.chats[other.active].created_at, created_at, "selection follows the chat, not its shifted id");
    });
}

/// The new window's composer starts empty; selecting the opened chat must
/// not stash that emptiness over the previously-active chat's saved draft.
#[test]
fn open_in_new_window_preserves_other_draft() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let id = cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx); // chat 1 becomes active
            this.chats[0].draft = "keep me".into();
            this.select_chat(0, window, cx); // back to chat 0 — its draft loads into the composer
            this.chats[1].id // open chat 1 in the new window
        })
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.open_chat_in_new_window(id, cx)));
    app.run_until_parked();

    let others = other_workspaces(&mut app, &ws);
    assert_eq!(others.len(), 1);
    app.read(|cx| {
        let other = others[0].read(cx);
        assert_eq!(other.active, 1, "chat 1 is selected");
        assert_eq!(other.chats[0].draft, "keep me", "the outgoing chat's draft survives the switch");
    });
}

/// The ⋯ menu on the chat titlebar offers "Open in New Window".
#[test]
fn chat_menu_lists_open_in_new_window() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
        assert!(
            snapshots(window)
                .iter()
                .any(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some("Open in New Window")),
            "chat menu should offer Open in New Window"
        );
    });
}

/// Right-clicking a sidebar row offers "Open in New Window" for that row's
/// chat — clicking it spawns the window with that chat selected, even when
/// the row isn't the active chat.
#[test]
fn row_menu_opens_chat_in_new_window() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (row_id, created_at) = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx); // chats[1] becomes active
            (this.chats[0].id, this.chats[0].created_at)
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("chat-row", row_id), cx);
    });
    // The menu entity is built in a deferred callback after this update.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the row menu");
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Open in New Window"))
            .unwrap_or_else(|| panic!("row menu should offer Open in New Window"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
    });
    app.run_until_parked();

    let others = other_workspaces(&mut app, &ws);
    assert_eq!(others.len(), 1, "clicking the item spawns a window");
    app.read(|cx| {
        let other = others[0].read(cx);
        assert_eq!(other.chats.len(), 2, "the chat is shared, not duplicated");
        assert_eq!(other.chats[other.active].created_at, created_at, "the row's chat is selected, not the previously active one");
    });
}

/// Dragging a sidebar row onto the chat pane tears the chat off into its
/// own window — the `chat-pane` drop target feeds the same
/// `open_chat_in_new_window` path as the menu item, so the spawned window
/// binds the project and selects the dragged chat.
#[test]
fn drag_chat_row_onto_pane_tears_off() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let (id, created_at) = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_chat(cx);
            (this.chats[1].id, this.chats[1].created_at)
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.drag_to(("chat-row", id), "chat-pane", cx);
        assert!(!ws.read(cx).tear_off_hover, "the drop clears the hover flag");
    });
    app.run_until_parked();

    let others = other_workspaces(&mut app, &ws);
    assert_eq!(others.len(), 1, "dropping a chat on the pane opens a window");
    app.read(|cx| {
        let other = others[0].read(cx);
        assert_eq!(other.project.root(), ws.read(cx).project.root(), "new window binds the same project");
        assert_eq!(other.chats.len(), 2, "the chat is shared, not duplicated");
        assert_eq!(other.chats[other.active].created_at, created_at, "the dragged chat is selected");
    });
}

/// A temporary chat never reaches disk, so another window couldn't load
/// it — dropping its row on the pane must be a no-op.
#[test]
fn drag_temp_chat_onto_pane_spawns_nothing() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let temp_id = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            this.chats.iter().find(|c| c.ephemeral).unwrap().id
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.drag_to(("chat-row", temp_id), "chat-pane", cx);
    });
    app.run_until_parked();
    assert!(other_workspaces(&mut app, &ws).is_empty(), "ephemeral chats can't tear off");
}
