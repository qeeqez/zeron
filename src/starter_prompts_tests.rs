//! Headless tests for the empty state's starter-prompt chips: they render
//! only while the chat is empty and a provider is usable, a click loads the
//! canned prompt into the composer and sends it through the normal path,
//! and the onboarding card still wins when no provider is configured. Same
//! harness as `onboarding_tests.rs` — declared via `#[path]` in
//! `views::mod` so `main.rs` stays under the SLOC cap.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::model::{MessageKind, Role};
use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-starters-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
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

/// A backend that records the prompt it was sent and finishes immediately —
/// deterministic, unlike `SimBackend`, which fails replies at random.
struct RecordingBackend {
    prompts: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        self.prompts.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::TextDelta("done".into()));
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Point the workspace at `backend` with a model selected — `start_reply`
/// refuses to run without one.
fn use_backend(ws: &Entity<Workspace>, backend: impl AgentBackend + 'static, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.backend = std::sync::Arc::new(backend);
            this.model = "m".into();
        });
    });
}

/// Drop every provider instance — the state a first-run user is in.
fn remove_all_providers(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ws.update(cx, |w, cx| {
            let ids: Vec<String> = w.provider_instances().iter().map(|p| p.id.clone()).collect();
            for id in ids {
                w.remove_provider(&id, cx);
            }
        });
    });
}

/// Advance the test clock until `cond` holds or the budget runs out.
fn until(ws: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if ws.read_with(cx, |w, _| cond(w)) {
            return;
        }
    }
    panic!("condition never held");
}

#[test]
fn chips_render_when_provider_configured() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("starter-prompts").visible(), "empty chat should offer starter chips");
        for id in ["starter-explain", "starter-fix-bug", "starter-add-tests", "starter-refactor"] {
            assert!(window.find(id).visible(), "chip {id} should render");
        }
        assert!(window.try_find("onboarding-card").is_none(), "a configured provider hides the card");
    });
}

#[test]
fn chip_click_sends_canned_prompt() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    use_backend(&ws, RecordingBackend { prompts: prompts.clone() }, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("starter-explain", cx);
    });
    until(&ws, cx, |w| !w.chats[0].running);
    ws.read_with(cx, |w, _| {
        let msgs = &w.chats[0].messages;
        assert_eq!(msgs[0].role, Role::User, "the chip's prompt lands as a user message");
        assert!(
            matches!(&msgs[0].kind, MessageKind::Text(t) if t.as_ref() == "Explain the structure of this codebase and what it does."),
            "the canned prompt is the sent text"
        );
    });
    assert_eq!(
        prompts.lock().first().map(String::as_str),
        Some("Explain the structure of this codebase and what it does."),
        "the normal send path delivered the canned prompt to the backend"
    );
    assert!(ws.read_with(cx, |w, cx| w.composer.read(cx).value().trim().is_empty()), "send clears the composer");
    // The chat has messages now — the chips are gone on the next render.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("starter-prompts").is_none(), "chips disappear once the chat has messages");
    });
}

#[test]
fn chips_hidden_once_chat_has_messages() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("starter-prompts").visible());
        ws.update(cx, |w, cx| {
            w.push_user_message(Queued::new("typed earlier".into(), vec![]), window, cx);
        });
        window.draw(cx).clear(cx);
        assert!(window.try_find("starter-prompts").is_none(), "a non-empty chat shows the transcript, not chips");
    });
}

#[test]
fn onboarding_card_wins_without_provider() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    remove_all_providers(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("onboarding-card").visible(), "no usable provider shows the card");
        assert!(window.try_find("starter-prompts").is_none(), "the card replaces the chips");
    });
}
