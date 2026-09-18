//! Tests for message versioning: a regenerate/retry moves the outgoing
//! reply into the new reply's `alternatives` (newest-first), the footer's
//! `< N/M >` pager swaps versions back in, the chain round-trips through
//! `persist`, and truncating a versioned reply drops its chain. Declared
//! from `chat_delete.rs` via `#[path]` — `main.rs` is at the SLOC cap.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{Chat, ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// A backend that answers each prompt with a distinct one-line reply —
/// `reply N` for the Nth send — then finishes the turn.
struct StubBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for StubBackend {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        let mut sent = self.sent.lock();
        sent.push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta(format!("reply {}", sent.len()).into()));
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-version-test-{}", std::process::id()));
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

/// Point the chat's turns at a temp workdir on the stub backend — keeps
/// checkpoint snapshots out of the real repo.
fn use_stub_backend(
    ws: &Entity<Workspace>, cx: &mut VisualTestContext, workdir: &std::path::Path,
) -> std::sync::Arc<parking_lot::Mutex<Vec<String>>> {
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    ws.update(cx, |this, _| {
        this.backend = std::sync::Arc::new(StubBackend { sent: sent.clone() });
        this.chats[this.active].workdir = workdir.to_string_lossy().into_owned();
    });
    sent
}

/// A temp workdir for the chat's turns.
fn temp_workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-version-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Push a message without starting a reply.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            alternatives: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// The text of message `ix` — panics on a non-text message.
fn text_at(ws: &Entity<Workspace>, cx: &VisualTestContext, ix: usize) -> String {
    ws.read_with(cx, |this, _| match &this.chats[this.active].messages[ix].kind {
        MessageKind::Text(t) => t.to_string(),
        _ => panic!("message {ix} is not text"),
    })
}

#[test]
fn regenerate_keeps_the_old_reply_as_an_alternative() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("regen");
    let _sent = use_stub_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    cx.run_until_parked();
    ws.read_with(cx, |this, _| {
        let chat = &this.chats[0];
        assert_eq!(chat.messages.len(), 2, "the new reply replaced the old one");
        let reply = &chat.messages[1];
        assert!(matches!(&reply.kind, MessageKind::Text(t) if t.as_str() == "reply 1"));
        assert_eq!(reply.alternatives.len(), 1, "the outgoing reply became an alternative");
        assert!(
            matches!(&reply.alternatives[0].kind, MessageKind::Text(t) if t.as_str() == "reply one"),
            "the alternative holds the replaced reply"
        );
        assert_eq!(reply.version_position(), 1, "the fresh reply is the newest version");
        assert!(chat.pending_alternatives.is_empty(), "the chain moved onto the reply");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn retry_last_versions_the_reply() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("retry");
    let _sent = use_stub_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    ws.update(cx, |this, cx| this.retry_last(cx));
    cx.run_until_parked();
    ws.read_with(cx, |this, _| {
        let reply = &this.chats[0].messages[1];
        assert!(matches!(&reply.kind, MessageKind::Text(t) if t.as_str() == "reply 1"));
        assert_eq!(reply.alternatives.len(), 1);
        assert!(matches!(&reply.alternatives[0].kind, MessageKind::Text(t) if t.as_str() == "reply one"));
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn pager_swaps_versions_and_reports_position() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("pager");
    let _sent = use_stub_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The pager sits only on the versioned assistant row — the user
        // row and unversioned replies never get one.
        assert!(window.try_find(("ver-pos", 0usize)).is_none(), "user rows have no pager");
        assert_eq!(window.find(("ver-pos", 1usize)).label(), Some("1/2"), "the fresh reply is the newest version");
        window.click(("ver-prev", 1usize), cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find(("ver-pos", 1usize)).label(), Some("2/2"), "paging back reaches the old reply");
    });
    assert_eq!(text_at(&ws, cx, 1), "reply one", "the old reply is live again");
    ws.read_with(cx, |this, _| {
        let reply = &this.chats[0].messages[1];
        assert_eq!(reply.alternatives.len(), 1);
        assert!(
            matches!(&reply.alternatives[0].kind, MessageKind::Text(t) if t.as_str() == "reply 1"),
            "the regenerated reply moved into the chain"
        );
    });
    cx.update(|window, cx| {
        window.click(("ver-next", 1usize), cx);
        window.draw(cx).clear(cx);
        assert_eq!(window.find(("ver-pos", 1usize)).label(), Some("1/2"), "paging forward returns to the newest");
    });
    assert_eq!(text_at(&ws, cx, 1), "reply 1", "the regenerated reply is live again");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn alternatives_survive_save_load() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("persist");
    let _sent = use_stub_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    cx.run_until_parked();
    let dir = std::env::temp_dir().join(format!("rixlcode-version-chats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let chat = ws.read_with(cx, |this, _| {
        let mut chat = Chat::new(0, this.chats[0].title.clone());
        chat.messages = this.chats[0].messages.clone();
        chat
    });
    crate::persist::save_chats(&dir, &[chat]);
    let mut next_id = 0;
    let mut loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    crate::persist::hydrate_all(&mut loaded, &dir);
    assert_eq!(loaded.len(), 1);
    let reply = &loaded[0].messages[1];
    assert!(matches!(&reply.kind, MessageKind::Text(t) if t.as_str() == "reply 1"));
    assert_eq!(reply.alternatives.len(), 1, "the version chain persisted");
    assert!(matches!(&reply.alternatives[0].kind, MessageKind::Text(t) if t.as_str() == "reply one"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn truncating_a_versioned_reply_drops_its_chain() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("truncate");
    let _sent = use_stub_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    push(&ws, cx, Role::User, "second");
    push(&ws, cx, Role::Assistant, "reply two");
    // Version the second reply, then regenerate the first — the second's
    // whole chain drops with the truncated tail.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(3, window, cx));
    });
    cx.run_until_parked();
    ws.read_with(cx, |this, _| {
        assert_eq!(this.chats[0].messages[3].alternatives.len(), 1, "the second reply is versioned");
    });
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.regenerate_from(1, window, cx));
    });
    cx.simulate_prompt_answer("Regenerate");
    cx.run_until_parked();
    ws.read_with(cx, |this, _| {
        let chat = &this.chats[0];
        assert_eq!(chat.messages.len(), 2, "the tail — versioned reply included — is gone");
        // The new reply keeps only the reply it replaced — the dropped
        // tail's chain never leaks into a survivor's alternatives.
        let reply = &chat.messages[1];
        assert_eq!(reply.alternatives.len(), 1);
        assert!(matches!(&reply.alternatives[0].kind, MessageKind::Text(t) if t.as_str() == "reply one"));
    });
    let _ = std::fs::remove_dir_all(&workdir);
}
