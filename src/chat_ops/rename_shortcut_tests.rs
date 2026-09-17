//! Headless tests for the sidebar's Enter-to-rename shortcut: a Cmd-clicked
//! row hands the keyboard to the sidebar, Enter opens the inline editor with
//! the title selected, Enter commits and Escape cancels — and the shortcut
//! stays out of the composer's way. Mount pattern matches `select_tests.rs`;
//! helpers are duplicated because sibling test files can't share them.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    App, AppContext, ElementId, Entity, Focusable, InputEvent, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, Window,
};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-renamesc-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process.
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

/// `n` extra chats on top of the one the workspace opens with — returns
/// every chat id in `chats` order.
fn chats(ws: &Entity<Workspace>, n: usize, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            for _ in 0..n {
                this.new_chat(cx);
            }
            this.chats.iter().map(|c| c.id).collect()
        })
    })
}

/// `window.click` with held modifiers — the test API's click always sends
/// none, so a ⌘-click needs raw down/up events on the target's center.
fn click_with(window: &mut Window, id: impl Into<ElementId>, modifiers: Modifiers, cx: &mut App) {
    window.render_frame(cx);
    let position = window.find(id).bounds().center();
    window.dispatch_event(
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    window.dispatch_event(
        MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn cmd() -> Modifiers {
    Modifiers { platform: true, ..Default::default() }
}

fn sidebar_focused(ws: &Entity<Workspace>, window: &Window, cx: &mut App) -> bool {
    ws.update(cx, |ws, _| ws.sidebar_focus.is_focused(window))
}

fn composer_focused(ws: &Entity<Workspace>, window: &Window, cx: &mut App) -> bool {
    ws.update(cx, |ws, cx| ws.composer.read(cx).focus_handle(cx).is_focused(window))
}

/// Cmd-click selects the row and focuses the sidebar; Enter opens the row's
/// inline editor seeded with the title fully selected, so typing replaces it
/// and Enter commits.
#[test]
fn enter_renames_the_selected_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, _cx| this.chats[0].title = "Keep me".into());
        window.draw(cx).clear(cx);

        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        assert!(sidebar_focused(&ws, window, cx), "cmd-click hands the keyboard to the sidebar");
        assert!(!composer_focused(&ws, window, cx));

        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).renaming, Some(ids[0]), "Enter should start the inline rename");
        assert!(window.find(("rename-input", ids[0])).visible(), "editor should replace the title");
        assert_eq!(ws.read(cx).rename.read(cx).value().to_string(), "Keep me", "editor is seeded with the title");
    });
    // The deferred focus lands between updates; typing then replaces the
    // selected title.
    cx.update(|window, cx| {
        window.input("Renamed", cx);
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].title.as_ref(), "Renamed", "Enter should commit the rename");
        assert_eq!(ws.read(cx).renaming, None, "Enter should end the edit");
        assert!(composer_focused(&ws, window, cx), "focus should return to the composer after Enter");
    });
}

/// Escape abandons the edit — the title stays and focus returns to the
/// composer.
#[test]
fn escape_cancels_the_shortcut_rename() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        ws.update(cx, |this, _cx| this.chats[0].title = "Keep me".into());
        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("rename-input", ids[0])).visible());
    });
    cx.update(|window, cx| {
        window.input("Discard", cx);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).chats[0].title.as_ref(), "Keep me", "Escape must not rename");
        assert_eq!(ws.read(cx).renaming, None, "Escape should end the edit");
        assert!(composer_focused(&ws, window, cx), "focus should return to the composer after Escape");
    });
}

/// The "sidebar" context keeps the binding off while an input owns the keys:
/// Enter in the composer sends/types, never renames — even with a row
/// selected.
#[test]
fn enter_does_not_rename_while_composer_focused() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    crate::composer_testutil::use_sim(&ws, cx);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        // Select the row without moving focus — the composer keeps the keys.
        ws.update(cx, |this, cx| this.toggle_chat_selection(ids[0], cx));
        focus_composer(&ws, window, cx);
        assert!(composer_focused(&ws, window, cx));

        window.input("hello", cx);
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).renaming, None, "Enter in the composer must not open rename");
        assert!(window.try_find(("rename-input", ids[0])).is_none(), "no editor mounted");
    });
    // PressEnter lands on the effect flush — the send runs after it.
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert!(
            ws.read(cx).chats[ws.read(cx).active]
                .messages
                .iter()
                .any(|m| matches!(&m.kind, crate::model::MessageKind::Text(t) if t.contains("hello"))),
            "Enter sent the typed message instead"
        );
    });
}

/// A plain click selects the chat and keeps the composer focused — the
/// sidebar's focusable wrap must not strand the keyboard on the row.
#[test]
fn plain_click_keeps_composer_focus() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        window.click(("chat-row", ids[0]), cx);
        window.draw(cx).clear(cx);
        assert!(composer_focused(&ws, window, cx), "a plain row click must leave the composer focused");
        assert!(!sidebar_focused(&ws, window, cx));
    });
}

/// With two rows selected there is no single rename target — Enter
/// propagates instead of picking one.
#[test]
fn enter_does_not_rename_a_multi_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        click_with(window, ("chat-row", ids[1]), cmd(), cx);
        assert!(sidebar_focused(&ws, window, cx));
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).renaming, None, "Enter must not rename a multi-selection");
    });
}

/// Focus the composer without disturbing the selection — the mount helper
/// doesn't guarantee it.
fn focus_composer(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.composer.update(cx, |s, cx| s.focus(window, cx));
    });
}
