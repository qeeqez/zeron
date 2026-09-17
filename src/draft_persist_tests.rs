//! Headless tests for composer-draft persistence: typing writes the draft
//! through to the chat, switching chats stashes and restores it, the 1s
//! ticker's debounce saves it mid-session, sending clears it, and a
//! relaunch seeds the composer from the saved draft. Disk round-trips live
//! in `persist_tests.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::composer_testutil::{composer_value, type_and_send, until, use_sim};
use crate::model::Chat;
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

/// Mount a `Workspace` in a headless window without wiping HOME first —
/// the restore test seeds chats before mounting, so it can't use
/// `composer_testutil::open_workspace` (which clears the dir).
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let window = cx.open_window(size(px(1024.), px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    let workspace = workspace.unwrap();
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |composer, cx| composer.focus(window, cx));
        });
    });
    (workspace, cx)
}

/// The chats dir the mounted workspace persists into.
fn chats_dir(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> std::path::PathBuf {
    workspace.read_with(cx, |ws, _| ws.project.chats_dir())
}

/// The active chat's persisted draft field.
fn draft(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    workspace.read_with(cx, |ws, _| ws.chats[ws.active].draft.clone())
}

#[gpui_kit::test]
fn switch_away_stashes_and_persists_the_draft(cx: &mut TestAppContext) {
    sandbox_home();
    let (ws, cx) = mount(cx);
    cx.run_until_parked();
    cx.update(|window, cx| window.input("draft one", cx));
    assert_eq!(draft(&ws, cx), "draft one", "typing writes through to the chat");

    cx.update(|_, cx| ws.update(cx, |ws, cx| ws.new_chat(cx)));
    cx.run_until_parked();
    assert_eq!(composer_value(&ws, cx), "", "the new chat starts empty");
    cx.update(|window, cx| window.input("draft two", cx));

    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.select_chat(0, window, cx)));
    assert_eq!(composer_value(&ws, cx), "draft one", "switching back restores the stash");
    let mut next_id = 99;
    let loaded = crate::persist::load_chats(&chats_dir(&ws, cx), &mut next_id, false);
    assert_eq!(loaded[0].draft, "draft one", "the switch's save persisted the stash");
    assert_eq!(loaded[1].draft, "draft two", "the outgoing chat's draft persisted too");
}

#[gpui_kit::test]
fn typing_saves_the_draft_after_a_debounce(cx: &mut TestAppContext) {
    sandbox_home();
    let (ws, cx) = mount(cx);
    cx.run_until_parked();
    let file = chats_dir(&ws, cx).join("0.json");
    cx.update(|window, cx| window.input("still typing", cx));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(!saved.contains("still typing"), "the debounce hasn't flushed yet");

    until(&ws, cx, |ws| ws.draft_save_ticks.is_none());
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("still typing"), "the debounced save wrote the draft: {saved}");
}

#[gpui_kit::test]
fn send_clears_the_draft(cx: &mut TestAppContext) {
    sandbox_home();
    let (ws, cx) = mount(cx);
    use_sim(&ws, cx);
    cx.run_until_parked();
    type_and_send(cx, "ship it");
    cx.run_until_parked();
    assert_eq!(composer_value(&ws, cx), "");
    assert_eq!(draft(&ws, cx), "", "sending clears the persisted draft field");
    let saved = std::fs::read_to_string(chats_dir(&ws, cx).join("0.json")).unwrap();
    assert!(!saved.contains("\"draft\""), "the send's save persisted the cleared draft: {saved}");
}

#[gpui_kit::test]
fn draft_restores_into_the_composer_on_launch(cx: &mut TestAppContext) {
    sandbox_home();
    // A previous session's store: one chat with an unsent draft.
    let mut chat = Chat::new(0, "prior chat");
    chat.draft = "was mid-sentence".into();
    let project = crate::project::Project::current();
    crate::persist::save_chats(&project.chats_dir(), &[chat]);
    project.save_state(&crate::project::ProjectState { active_chat: 0, ..Default::default() });

    let (ws, cx) = mount(cx);
    cx.run_until_parked();
    assert_eq!(composer_value(&ws, cx), "was mid-sentence", "the composer restores the saved draft");
}
