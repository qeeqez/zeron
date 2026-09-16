//! Headless tests for the message menu's "Regenerate with model" submenu:
//! it lists every enabled instance's effective models, a pick switches the
//! selection and re-runs the turn, picking the current model is a plain
//! retry, and a single-model workspace hides the submenu. Declared via
//! `#[path]` in `chat_msg.rs` — `main.rs` is at the SLOC cap. Same mount
//! harness as `views/message_tests.rs`.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext, Window};

use crate::backend::{AgentBackend, ReplyStream};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-retry-model-test-{}", std::process::id()));
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

/// Push a user message without starting a reply — retry tests seed the
/// prompt directly instead of going through `send`.
fn seed_user(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::User,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// Seed the active chat with a completed assistant turn.
fn seed_reply(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| this.push_note("reply body".into(), cx));
}

/// Records every (prompt, model) the workspace sends so a retry can be
/// asserted against the real `run_backend` path. The stream ends
/// immediately.
struct RecordingBackend {
    sent: std::sync::Arc<parking_lot::Mutex<Vec<(String, String)>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn send(&self, prompt: &str, model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        self.sent.lock().push((prompt.to_string(), model.to_string()));
        let (_tx, events) = std::sync::mpsc::channel();
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Right-click message `ix` and hover "Regenerate with model" so the
/// submenu's items are in the snapshot tree. The menu entity builds in a
/// deferred callback, so the click and the hover land in separate updates.
/// Panics when the item is missing — callers that expect it hidden assert
/// before calling.
fn open_regenerate_submenu(ix: usize, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.right_click(("msg", ix), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let item = snapshots(window)
            .iter()
            .find(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some("Regenerate with model"))
            .unwrap_or_else(|| panic!("menu should offer the model submenu"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").hover(id, cx);
        window.draw(cx).clear(cx);
    });
}

/// Menu-item labels in the snapshot tree — the MenuItem role filters out
/// the composer picker's button, whose "provider · model" label collides
/// with a submenu row's.
fn menu_item_labels(window: &Window) -> Vec<String> {
    snapshots(window)
        .iter()
        .filter(|s| s.role() == Some(A11yRole::MenuItem))
        .filter_map(|s| s.label().map(str::to_string))
        .collect()
}

/// Click the submenu item with `label` — panics when it isn't offered.
fn click_submenu_item(window: &mut Window, label: &str, cx: &mut App) {
    let item = snapshots(window)
        .iter()
        .find(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some(label))
        .unwrap_or_else(|| panic!("submenu should offer {label}"))
        .clone();
    let id = item.path().last().unwrap().clone();
    window.within("submenu").click(id, cx);
}

/// The submenu lists every enabled instance's models — "provider · model"
/// labels — drawn from the same effective lists the composer picker shows.
#[test]
fn regenerate_submenu_lists_catalog_models() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    open_regenerate_submenu(1, cx);
    cx.update(|window, _cx| {
        let labels = menu_item_labels(window);
        for expected in [
            "Codex · gpt-5-codex",
            "Codex · gpt-5",
            "Codex · gpt-5-mini",
            "Claude · Sonnet",
            "Claude · Opus",
            "Claude · Haiku",
            "Claude · Fable",
            "Sim · Sim",
        ] {
            assert!(labels.iter().any(|l| l == expected), "submenu should offer {expected}: {labels:?}");
        }
        // `checked` renders as a check icon, not an a11y flag — the
        // current-model pick is covered by the plain-retry test below.
    });
}

/// Picking another model on the same instance selects it and re-sends the
/// last user prompt through the backend — the send records the new model.
#[test]
fn regenerate_with_model_switches_and_resends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    ws.update(cx, |this, _cx| {
        this.backend = std::sync::Arc::new(RecordingBackend { sent: sent.clone() });
    });
    open_regenerate_submenu(1, cx);
    cx.update(|window, cx| {
        click_submenu_item(window, "Codex · gpt-5-mini", cx);
    });
    app.read(|cx| {
        let w = ws.read(cx);
        assert_eq!(w.selected_model(), "gpt-5-mini", "pick should switch the live model");
        assert_eq!(w.selected_provider(), Some("codex-cli"));
        let msgs = &w.chats[0].messages;
        assert_eq!(msgs.len(), 1, "stale assistant reply should be popped");
        assert!(matches!(msgs[0].role, Role::User));
    });
    assert_eq!(sent.lock().as_slice(), &[("fix the bug".to_string(), "gpt-5-mini".to_string())]);
}

/// Picking the current model is a plain retry — same re-send, no
/// selection change.
#[test]
fn regenerate_with_current_model_is_plain_retry() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let sent = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    ws.update(cx, |this, _cx| {
        this.backend = std::sync::Arc::new(RecordingBackend { sent: sent.clone() });
    });
    open_regenerate_submenu(1, cx);
    cx.update(|window, cx| {
        click_submenu_item(window, "Codex · gpt-5-codex", cx);
    });
    app.read(|cx| {
        let w = ws.read(cx);
        assert_eq!(w.selected_model(), "gpt-5-codex", "selection unchanged");
        assert_eq!(w.chats[0].messages.len(), 1, "stale reply popped for the retry");
    });
    assert_eq!(sent.lock().as_slice(), &[("fix the bug".to_string(), "gpt-5-codex".to_string())]);
}

/// A pick on another instance switches provider+model and still re-runs —
/// `sim` is the only cross-provider target tests can use without spawning
/// a real backend.
#[test]
fn regenerate_with_other_provider_switches_and_resends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    open_regenerate_submenu(1, cx);
    cx.update(|window, cx| {
        click_submenu_item(window, "Sim · Sim", cx);
    });
    app.read(|cx| {
        let w = ws.read(cx);
        assert_eq!(w.selected_provider(), Some("sim"), "pick should switch the instance");
        assert_eq!(w.selected_model(), "sim");
        // The retry ran through `simulate_reply`: the stale reply is gone
        // and the simulated tool call landed.
        let msgs = &w.chats[0].messages;
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[1].kind, MessageKind::Tool(_)), "sim retry should push its tool call");
    });
}

/// One switchable model means nothing to switch to — the submenu stays
/// out and "Retry" stands alone.
#[test]
fn single_model_hides_submenu() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, _cx| {
        for p in this.providers.iter_mut() {
            p.enabled = p.id == "codex-cli";
        }
        // A one-entry catalog — `apply_model_config` keeps unconfigured
        // catalog models, so trimming `models` alone can't shrink the list.
        this.model_catalog.insert(
            "codex-cli".into(),
            vec![crate::model::ModelInfo {
                id: "gpt-5-codex".into(),
                label: "gpt-5-codex".into(),
                ..Default::default()
            }],
        );
    });
    seed_user(&ws, "fix the bug", cx);
    seed_reply(&ws, cx);
    cx.update(|window, cx| {
        window.right_click(("msg", 1usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_item_labels(window);
        assert!(labels.iter().any(|l| l == "Retry"), "plain Retry stays: {labels:?}");
        assert!(!labels.iter().any(|l| l == "Regenerate with model"), "one model leaves nothing to switch to: {labels:?}");
    });
}
