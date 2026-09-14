//! Headless UI tests for conversation continuity: chats saved to the
//! project's store under `~/.rixl/rixlcode/projects/` reappear when a new
//! `Workspace` mounts, so a restart resumes prior conversations. Same
//! pattern as `ui_tests.rs` — `#[gpui_kit::test]` and a bare
//! `use gpui_kit::*` crash the proc-macro on this nightly.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};

use crate::model::{Chat, ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Redirect persistence into a throwaway dir so tests never read or write
/// the real `~/.rixl/rixlcode` settings and chats.
fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

/// Mount a `Workspace` in a headless window — the "launch" half of a restart.
fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &'static mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let window = cx.open_window(size(px(1024.), px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    (workspace.unwrap(), cx)
}

#[test]
fn saved_chats_reopen_on_launch() {
    sandbox_home();
    // A previous session's store: two chats, the second one active.
    let mut first = Chat::new(0, "prior chat");
    first.messages = std::rc::Rc::new(vec![ChatMessage {
        role: Role::User,
        kind: MessageKind::Text("earlier question".into()),
        rating: None,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }]);
    let project = crate::project::Project::current();
    crate::persist::save_chats(&project.chats_dir(), &[first, Chat::new(1, "second chat")]);
    project.save_state(&crate::project::ProjectState { active_chat: 1 });

    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("sidebar-wrap").visible(), "sidebar renders");
        let ws = ws.read(cx);
        assert_eq!(ws.chats.len(), 2, "both saved chats must be restored");
        assert_eq!(ws.chats[0].title, "prior chat");
        assert_eq!(ws.chats[1].title, "second chat");
        assert_eq!(ws.active, 1, "the previously active chat reopens");
        assert!(matches!(&ws.chats[0].messages[0].kind, MessageKind::Text(t) if t.as_str() == "earlier question"));
        assert!(ws.next_chat_id >= 2, "new chats must not reuse restored ids");
    });
}

#[test]
fn sent_message_and_reply_survive_reload() {
    sandbox_home();
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    // First "session": send a message and let the sim backend finish.
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.backend = std::sync::Arc::new(crate::backend::SimBackend);
            ws.composer.update(cx, |composer, cx| composer.set_value("remember this", window, cx));
            ws.send(window, cx);
        });
    });
    for _ in 0..32 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if !ws.read_with(cx, |ws, _| ws.chats[0].running) {
            break;
        }
    }
    assert!(!ws.read_with(cx, |ws, _| ws.chats[0].running), "simulated reply never finished");

    // Second "session": a fresh Workspace over the same HOME must restore
    // the transcript — user message plus the completed assistant reply.
    let (ws2, cx2) = open_workspace(&mut app);
    cx2.update(|_window, cx| {
        let ws = ws2.read(cx);
        assert_eq!(ws.chats.len(), 1);
        let roles: Vec<Role> = ws.chats[0].messages.iter().map(|m| m.role).collect();
        assert!(roles.contains(&Role::User), "user message must persist");
        assert!(roles.contains(&Role::Assistant), "assistant reply must persist");
        assert!(
            ws.chats[0]
                .messages
                .iter()
                .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.as_str() == "remember this"))
        );
        assert!(!ws.chats[0].running);
    });
}
