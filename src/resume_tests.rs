//! Headless tests for session resume: the sidebar's Resume section lists
//! past threads, picking one opens a chat bound to the thread id, and the
//! next send continues that thread. A recording fake stands in for the
//! backend — no real `codex app-server` is spawned.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, SessionInfo, TurnContext};
use crate::workspace::Workspace;

/// A backend with session support: records each turn's `TurnContext` and
/// serves a canned session list — the codex assertions without a real
/// `codex app-server`.
struct SessionBackend {
    ctxs: std::sync::Arc<parking_lot::Mutex<Vec<TurnContext>>>,
    sessions: Vec<SessionInfo>,
}

impl AgentBackend for SessionBackend {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn models(&self) -> Vec<crate::model::ModelInfo> {
        vec![crate::model::ModelInfo {
            id: "m".into(),
            label: "M".into(),
            description: "".into(),
            ..Default::default()
        }]
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, ctx: &TurnContext) -> ReplyStream {
        self.ctxs.lock().push(ctx.clone());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn supports_sessions(&self) -> bool {
        true
    }

    fn list_sessions(&self) -> Option<Vec<SessionInfo>> {
        Some(self.sessions.clone())
    }
}

fn session(id: &str, title: &str) -> SessionInfo {
    SessionInfo {
        id: id.into(),
        title: title.into(),
        updated: 1_789_382_089,
        cwd: "/tmp/proj".into(),
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-resume-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

#[test]
fn non_codex_backends_have_no_sessions() {
    assert!(!crate::backend::SimBackend.supports_sessions());
    assert!(crate::backend::SimBackend.list_sessions().is_none());
    assert!(crate::backend::SimBackend.resume_session("t").is_none());
    assert!(!crate::backend::ClaudeCliBackend::new(Vec::new()).supports_sessions());
    assert!(crate::backend::ClaudeCliBackend::new(Vec::new()).list_sessions().is_none());
}

#[test]
fn resume_row_hidden_without_session_support() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "hidden");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The default mount's backend is whatever the temp profile selects
        // — force a session-less one so the affordance must hide.
        ws.update(cx, |this, _| {
            this.backend = std::sync::Arc::new(crate::backend::SimBackend);
        });
        window.draw(cx).clear(cx);
        assert!(window.try_find("resume-toggle").is_none(), "session-less backend must hide the Resume row");
    });
}

#[test]
fn resume_section_lists_sessions_and_selecting_binds_the_thread() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "pick");
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(SessionBackend {
                ctxs: ctxs.clone(),
                sessions: vec![session("tid-1", "fix the parser"), session("tid-2", "add tests")],
            });
            // Test builds skip the fetch — land the canned list directly.
            let sessions = this.backend.list_sessions();
            this.land_sessions(sessions, cx);
            this.resume_open = true;
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("resume-session", 0usize)).visible(), "first session row should render");
        assert!(window.find(("resume-session", 1usize)).visible(), "second session row should render");

        window.click(("resume-session", 0usize), cx);
        window.draw(cx).clear(cx);
        ws.update(cx, |this, _| {
            let chat = &this.chats[this.active];
            assert_eq!(chat.thread_id, "tid-1", "the chat binds the session's thread id");
            assert_eq!(chat.title.as_ref(), "fix the parser");
        });

        // The next send continues the resumed thread.
        ws.update(cx, |this, cx| {
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("continue", window, cx));
            this.send(window, cx);
        });
    });
    let ctxs = ctxs.lock();
    assert_eq!(ctxs.len(), 1);
    assert_eq!(ctxs[0].thread_id.as_deref(), Some("tid-1"));
}

#[test]
fn opening_a_bound_session_selects_its_existing_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "reopen");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(SessionBackend { ctxs: Default::default(), sessions: vec![] });
            this.open_session(&session("tid-1", "first"), window, cx);
            let bound = this.active;
            this.new_chat(cx);
            assert_ne!(this.active, bound);
            // Picking the same session again selects the bound chat instead
            // of opening a duplicate.
            this.open_session(&session("tid-1", "first"), window, cx);
            assert_eq!(this.active, bound);
            assert_eq!(this.chats.iter().filter(|c| c.thread_id == "tid-1").count(), 1);
        });
    });
}

#[test]
fn bound_thread_survives_save_load_send() {
    // The resume chain end to end: a turn binds the backend's thread id,
    // the binding persists, and a relaunched workspace's next send carries
    // it on the TurnContext so the backend resumes instead of forking.
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "cycle");
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(SessionBackend { ctxs: ctxs.clone(), sessions: vec![] });
            let chat_id = this.chats[this.active].id;
            this.apply_event(chat_id, AgentEvent::ThreadBound("tid-9".into()), cx);
            assert_eq!(this.chats[this.active].thread_id, "tid-9", "the turn bound the thread");
            this.save();
        });
    });
    // Relaunch: reload the chats dir into a fresh chat set.
    let dir = cx.update(|_, cx| ws.read(cx).project.chats_dir());
    let mut next_id = 100;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].thread_id, "tid-9", "the binding round-trips through disk");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.chats = loaded;
            this.active = 0;
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("continue", window, cx));
            this.send(window, cx);
        });
    });
    let ctxs = ctxs.lock();
    assert_eq!(ctxs.len(), 1);
    assert_eq!(ctxs[0].thread_id.as_deref(), Some("tid-9"), "the reloaded chat resumes its thread");
}
