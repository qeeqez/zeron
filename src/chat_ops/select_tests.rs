//! Headless UI tests for the sidebar's multi-select + bulk ops: Cmd-click
//! toggles rows into `selected_chats`, the bar archives or deletes the set
//! behind one confirm, Esc and plain clicks drop the selection, and a chat
//! with a reply in flight is skipped by Delete. Mount pattern matches
//! `sidebar_dnd_tests.rs`; the ⌘-click helper mirrors `diff_open_tests.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    App, AppContext, ElementId, Entity, InputEvent, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, Window,
};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-select-test-{}", std::process::id()));
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

/// Cmd-click toggles a row into the selection set and back out; the bar
/// appears only while the set is non-empty.
#[test]
fn cmd_click_toggles_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("chat-selection-bar").is_none(), "no bar before any selection");

        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        click_with(window, ("chat-row", ids[1]), cmd(), cx);
        assert_eq!(
            ws.read(cx).selected_chats.iter().copied().collect::<std::collections::HashSet<_>>(),
            [ids[0], ids[1]].into_iter().collect(),
            "cmd-click adds each row to the set"
        );
        window.draw(cx).clear(cx);
        assert!(window.find("chat-selection-bar").visible(), "the bar appears with a selection");

        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        assert_eq!(ws.read(cx).selected_chats.iter().copied().collect::<Vec<_>>(), [ids[1]], "cmd-click on a selected row removes it");
    });
}

/// A plain click on a row drops the selection — including on the active
/// chat, where `select_chat` early-returns.
#[test]
fn plain_click_clears_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        assert_eq!(ws.read(cx).selected_chats.len(), 1);

        // ids[0] is not active (the newest chat is) — a plain click selects
        // it and drops the set.
        window.click(("chat-row", ids[0]), cx);
        assert!(ws.read(cx).selected_chats.is_empty(), "plain click clears the set");
        assert_eq!(ws.read(cx).chats[ws.read(cx).active].id, ids[0], "the click still selects the chat");

        // Same on the now-active row: the click can't reach `select_chat`'s
        // early return with the set intact.
        click_with(window, ("chat-row", ids[1]), cmd(), cx);
        window.click(("chat-row", ids[0]), cx);
        assert!(ws.read(cx).selected_chats.is_empty(), "clicking the active row clears the set");
    });
}

/// Archive applies to every selected chat and drops the set.
#[test]
fn archive_selected_archives_each() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        for id in &ids[..2] {
            click_with(window, ("chat-row", *id), cmd(), cx);
        }
        window.click("archive-selected", cx);
        window.draw(cx).clear(cx);
        let chats = &ws.read(cx).chats;
        assert!(chats.iter().find(|c| c.id == ids[0]).unwrap().archived, "first chat archived");
        assert!(chats.iter().find(|c| c.id == ids[1]).unwrap().archived, "second chat archived");
        assert!(!chats.iter().find(|c| c.id == ids[2]).unwrap().archived, "unselected chat untouched");
        assert!(ws.read(cx).selected_chats.is_empty(), "the op consumes the selection");
        assert!(window.try_find("chat-selection-bar").is_none(), "the bar hides once the set is empty");
    });
}

/// Archiving the active chat moves the selection to a live one — the same
/// fixup the single-chat Archive runs.
#[test]
fn archive_selected_moves_off_archived_active() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    // ids[1] is the active chat (newest); select it and archive.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_with(window, ("chat-row", ids[1]), cmd(), cx);
        window.click("archive-selected", cx);
        let ws = ws.read(cx);
        assert!(ws.chats.iter().find(|c| c.id == ids[1]).unwrap().archived);
        assert_eq!(ws.chats[ws.active].id, ids[0], "active moved to the first live chat");
    });
}

/// Delete asks once for the whole set, then removes every selected chat.
#[test]
fn delete_selected_confirms_once() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        for id in &ids[..2] {
            click_with(window, ("chat-row", *id), cmd(), cx);
        }
        window.click("delete-selected", cx);
    });
    assert!(app.has_pending_prompt(), "the bulk delete asks for confirmation");
    app.simulate_prompt_answer("Delete");
    app.run_until_parked();
    app.read(|cx| {
        let remaining: Vec<u64> = ws.read(cx).chats.iter().map(|c| c.id).collect();
        assert_eq!(remaining, [ids[2]], "both selected chats are gone, the unselected one stays");
        assert!(ws.read(cx).selected_chats.is_empty(), "the op consumes the selection");
    });
    assert!(!app.has_pending_prompt(), "one confirm covered the whole set");
}

/// Cancelling the confirm leaves every chat in place.
#[test]
fn delete_selected_cancel_keeps_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        window.click("delete-selected", cx);
    });
    app.simulate_prompt_answer("Cancel");
    app.run_until_parked();
    app.read(|cx| {
        assert_eq!(ws.read(cx).chats.len(), 2, "cancel deletes nothing");
    });
}

/// A chat with a reply in flight is skipped by bulk Delete — the confirm
/// covers only the deletable rows and the running chat survives.
#[test]
fn delete_selected_skips_running() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    ws.update(cx, |this, _| {
        this.chats.iter_mut().find(|c| c.id == ids[1]).unwrap().running = true;
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        for id in &ids {
            click_with(window, ("chat-row", *id), cmd(), cx);
        }
        window.click("delete-selected", cx);
    });
    app.simulate_prompt_answer("Delete");
    app.run_until_parked();
    app.read(|cx| {
        let remaining: Vec<u64> = ws.read(cx).chats.iter().map(|c| c.id).collect();
        assert_eq!(remaining, [ids[1]], "only the running chat survives the delete");
    });
}

/// Esc drops the selection without touching the chats.
#[test]
fn escape_clears_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);
        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        assert_eq!(ws.read(cx).selected_chats.len(), 1);
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).selected_chats.is_empty(), "Esc clears the set");
        assert!(window.try_find("chat-selection-bar").is_none(), "the bar hides");
    });
}

/// The bar's Clear button empties the set without touching the chats.
#[test]
fn clear_button_drops_selection() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_with(window, ("chat-row", ids[0]), cmd(), cx);
        window.click("clear-selected", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).selected_chats.is_empty(), "Clear empties the set");
        assert_eq!(ws.read(cx).chats.len(), 2, "no chat was touched");
        assert!(window.try_find("chat-selection-bar").is_none(), "the bar hides");
    });
}
