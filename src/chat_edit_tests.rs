//! Tests for edit-and-resend: opening a user message in the inline editor
//! leaves the transcript alone, committing truncates after it and resends
//! the edited text as a fresh turn (checkpointing the workdir first), and
//! cancelling keeps everything.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// A backend that writes `agent.txt` into the turn's workdir (like a real
/// agent edit), records the prompt, then finishes — no subprocess.
struct WriteBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for WriteBackend {
    fn name(&self) -> &'static str {
        "write"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, ctx: &crate::backend::TurnContext) -> ReplyStream {
        std::fs::write(ctx.cwd.join("agent.txt"), "agent").unwrap();
        self.sent.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
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
    let dir = std::env::temp_dir().join(format!("rixlcode-edit-test-{}", std::process::id()));
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

/// A temp workdir for the chat's turns — keeps the backend's `agent.txt`
/// and checkpoint snapshots out of the real repo.
fn temp_workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-edit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Point the chat's turns at `workdir` on the recording backend.
fn use_write_backend(
    ws: &Entity<Workspace>, cx: &mut VisualTestContext, workdir: &std::path::Path,
) -> std::sync::Arc<parking_lot::Mutex<Vec<String>>> {
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    ws.update(cx, |this, _| {
        this.backend = std::sync::Arc::new(WriteBackend { sent: sent.clone() });
        this.chats[this.active].workdir = workdir.to_string_lossy().into_owned();
    });
    sent
}

/// Push a message without starting a reply.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// Text of every `Text` message in the active chat, in order.
fn texts(ws: &Workspace) -> Vec<String> {
    ws.chats[ws.active]
        .messages
        .iter()
        .filter_map(|m| match &m.kind {
            MessageKind::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .collect()
}

/// The inline editor's current text.
fn editor_value(ws: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    ws.read_with(cx, |ws, app| ws.editing.as_ref().unwrap().input.read(app).value().to_string())
}

/// Overwrite the open editor's text.
fn set_editor(ws: &Entity<Workspace>, text: &str, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    ws.update(cx, |this, cx| {
        let input = this.editing.as_ref().unwrap().input.clone();
        input.update(cx, |s, cx| s.set_value(text, window, cx));
    });
}

#[test]
fn edit_opens_editor_without_truncating() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    push(&ws, cx, Role::User, "second");
    push(&ws, cx, Role::Assistant, "reply two");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.edit_message(0, window, cx));
    });
    ws.read_with(cx, |ws, _| {
        let edit = ws.editing.as_ref().expect("edit should be open");
        assert_eq!(edit.ix, 0);
        assert_eq!(ws.chats[ws.active].messages.len(), 4, "opening the editor must not truncate");
    });
    assert_eq!(editor_value(&ws, cx), "first", "editor is seeded with the message text");
}

#[test]
fn commit_truncates_and_resends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("commit");
    let sent = use_write_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    push(&ws, cx, Role::User, "second");
    push(&ws, cx, Role::Assistant, "reply two");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.edit_message(0, window, cx));
        set_editor(&ws, "first edited", window, cx);
        ws.update(cx, |this, cx| this.commit_edit(window, cx));
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.editing.is_none(), "commit closes the editor");
        assert_eq!(texts(ws), ["first edited"], "the old reply and later messages are dropped");
        assert!(ws.chats[ws.active].running, "the resend started a fresh turn");
    });
    assert_eq!(sent.lock().as_slice(), ["first edited"], "the edited text went to the backend");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn resend_checkpoints_the_workdir() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("ckpt");
    let _sent = use_write_backend(&ws, cx, &workdir);
    // Turn one runs for real — the backend writes into the workdir.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("first", window, cx));
            this.send(window, cx);
        });
    });
    assert!(workdir.join("agent.txt").exists(), "the first turn's edit landed");
    // Editing message 0 and resending must snapshot the workdir first —
    // the dropped turn's file lands in the new checkpoint.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.edit_message(0, window, cx));
        set_editor(&ws, "first edited", window, cx);
        ws.update(cx, |this, cx| this.commit_edit(window, cx));
    });
    ws.read_with(cx, |ws, _| {
        let chat = &ws.chats[ws.active];
        assert_eq!(chat.checkpoints.len(), 1, "the resend recorded a checkpoint");
        let crate::checkpoints::Checkpoint::Copy(dir) = &chat.checkpoints[0].checkpoint else {
            panic!("a plain workdir snapshots as a copy");
        };
        assert_eq!(std::fs::read_to_string(dir.join("agent.txt")).unwrap(), "agent", "the dropped turn's edits are recoverable");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn cancel_keeps_the_transcript() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.edit_message(0, window, cx));
        set_editor(&ws, "discarded", window, cx);
        ws.update(cx, |this, cx| this.cancel_edit(window, cx));
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.editing.is_none());
        assert_eq!(texts(ws), ["first", "reply one"], "cancel must not touch the transcript");
    });
}

#[test]
fn a_new_send_abandons_the_edit() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("send");
    let sent = use_write_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.edit_message(0, window, cx));
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("new message", window, cx));
            this.send(window, cx);
        });
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.editing.is_none(), "sending abandons the pending edit");
        assert_eq!(texts(ws), ["first", "reply one", "new message"], "the transcript was not truncated");
    });
    assert_eq!(sent.lock().as_slice(), ["new message"]);
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The hover toolbar shows an Edit affordance on user messages; clicking
/// it mounts the inline editor and Enter resends the edited text.
#[test]
fn edit_affordance_resends_from_the_row() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("affordance");
    let sent = use_write_backend(&ws, cx, &workdir);
    push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("edit", 0usize)).visible(), "Edit reveals on hover");
        window.click(("edit", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("msg-edit", 0usize)).visible(), "the bubble becomes an editor");
        assert!(window.find(("md-body", 1usize)).visible(), "the reply still renders");
    });
    // The deferred focus lands between updates; Enter then commits.
    cx.update(|window, cx| {
        set_editor(&ws, "first edited", window, cx);
        window.press("enter", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("msg-edit", 0usize)).is_none(), "the editor unmounts on commit");
        assert!(window.find(("md-body", 0usize)).visible(), "the edited message renders as a bubble");
    });
    assert_eq!(sent.lock().as_slice(), ["first edited"], "Enter resent the edited text");
    ws.read_with(cx, |ws, _| assert_eq!(texts(ws), ["first edited"]));
    let _ = std::fs::remove_dir_all(&workdir);
}
