//! Headless tests for AI chat titles: a fake backend stands in for the
//! agent — no real subprocess. Scripted sends replay canned event streams;
//! manual sends hand the test a channel so a rename can land mid-flight.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::workspace::Workspace;

type Prompts = std::sync::Arc<parking_lot::Mutex<Vec<String>>>;
type Sends = std::sync::Arc<parking_lot::Mutex<Vec<std::sync::mpsc::Sender<AgentEvent>>>>;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile. The
/// project points at a plain temp dir — checkpoint snapshots stay cheap
/// and no git refs land in the real repo.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-title-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    let ws = workspace.unwrap();
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.project = crate::project::Project::open(dir.join("proj"));
        });
    });
    (ws, cx)
}

/// A backend that records every prompt and answers `send` two ways: with
/// `scripts` non-empty each call replays the next script (the last one
/// repeats); with `scripts` empty each call pushes a channel onto `sends`
/// for the test to feed by hand.
struct FakeBackend {
    scripts: parking_lot::Mutex<std::collections::VecDeque<Vec<AgentEvent>>>,
    sends: Sends,
    prompts: Prompts,
}

impl FakeBackend {
    fn scripted(prompts: &Prompts, scripts: Vec<Vec<AgentEvent>>) -> Self {
        Self {
            scripts: parking_lot::Mutex::new(scripts.into()),
            sends: Default::default(),
            prompts: prompts.clone(),
        }
    }

    fn manual(prompts: &Prompts, sends: &Sends) -> Self {
        Self {
            scripts: Default::default(),
            sends: sends.clone(),
            prompts: prompts.clone(),
        }
    }
}

impl AgentBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        self.prompts.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        let mut scripts = self.scripts.lock();
        if scripts.is_empty() {
            self.sends.lock().push(tx);
        } else {
            let script = if scripts.len() > 1 {
                scripts.pop_front().unwrap_or_default()
            } else {
                scripts.front().cloned().unwrap_or_default()
            };
            for e in script {
                let _ = tx.send(e);
            }
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Point the workspace at `backend` with a model selected — sends and
/// title generation both refuse to run without one.
fn use_backend(ws: &Entity<Workspace>, backend: impl AgentBackend + 'static, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.backend = std::sync::Arc::new(backend);
            this.model = "m".into();
        });
    });
}

/// Type `text` into the composer and send it through the real `send` path.
fn submit(ws: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value(text, window, cx));
            this.send(window, cx);
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

/// Advance until `prompts` holds `n` entries — the title turn's `send`
/// happens inside a spawned task, so a bare `run_until_parked` can miss it.
fn until_prompts(prompts: &Prompts, n: usize, cx: &mut VisualTestContext) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if prompts.lock().len() >= n {
            return;
        }
    }
    panic!("never saw {n} prompts");
}

/// Give a would-be title task every chance to spawn and land.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
}

/// Commit a rename of chat `id` through the shared `commit_rename` path.
fn rename(ws: &Entity<Workspace>, id: u64, title: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.renaming = Some(id);
            this.rename_mode = crate::workspace::RenameMode::Dialog;
            this.rename.update(cx, |s, cx| s.set_value(title, window, cx));
            this.commit_rename(window, cx);
        });
    });
}

fn chat(ws: &Entity<Workspace>, cx: &VisualTestContext, f: impl Fn(&crate::model::Chat) -> bool) -> bool {
    ws.read_with(cx, |w, _| w.chats.get(w.active).is_some_and(f))
}

fn title_is(title: &'static str) -> impl Fn(&Workspace) -> bool {
    move |w| w.chats.get(w.active).is_some_and(|c| c.title == title)
}

/// The first completed exchange replaces the placeholder with the
/// backend's title — sanitized — and marks the chat so later turns don't
/// regenerate.
#[gpui_kit::test]
fn first_reply_titles_the_chat(cx: &mut TestAppContext) {
    let prompts: Prompts = Default::default();
    let (ws, cx) = mount(cx);
    use_backend(
        &ws,
        FakeBackend::scripted(
            &prompts,
            vec![
                vec![AgentEvent::TextStart, AgentEvent::TextDelta("Here's the fix.".into()), AgentEvent::Done],
                vec![AgentEvent::TextDelta("\"Fix the flaky test.\"\n".into()), AgentEvent::Done],
            ],
        ),
        cx,
    );
    submit(&ws, cx, "the login test flakes on CI");
    until(&ws, cx, title_is("Fix the flaky test"));
    assert!(chat(&ws, cx, |c| c.title_generated), "generated flag set");
    let sent = prompts.lock().clone();
    assert_eq!(sent.len(), 2, "chat turn plus title turn");
    assert!(sent[1].contains("the login test flakes on CI"), "title prompt carried the user message");
    assert!(sent[1].contains("Here's the fix."), "title prompt carried the assistant reply");
    // A second exchange must not send another title turn.
    submit(&ws, cx, "thanks");
    until(&ws, cx, |w| !w.chats[w.active].running);
    until_prompts(&prompts, 3, cx);
    settle(cx);
    assert_eq!(prompts.lock().len(), 3, "no second title turn");
    assert!(chat(&ws, cx, |c| c.title == "Fix the flaky test"), "generated title kept");
}

/// A chat renamed before its first send never triggers generation — the
/// title is already the user's.
#[gpui_kit::test]
fn manual_rename_before_send_skips_generation(cx: &mut TestAppContext) {
    let prompts: Prompts = Default::default();
    let (ws, cx) = mount(cx);
    use_backend(&ws, FakeBackend::scripted(&prompts, vec![vec![AgentEvent::TextDelta("ok".into()), AgentEvent::Done]]), cx);
    let id = ws.read_with(cx, |w, _| w.chats[w.active].id);
    rename(&ws, id, "My custom name", cx);
    submit(&ws, cx, "hello");
    until(&ws, cx, |w| !w.chats[w.active].running);
    settle(cx);
    assert_eq!(prompts.lock().len(), 1, "no title turn for a renamed chat");
    assert!(chat(&ws, cx, |c| c.title == "My custom name"), "rename kept");
    assert!(!chat(&ws, cx, |c| c.title_generated));
}

/// A rename committed while the title turn is in flight wins — the
/// generated title lands only when the placeholder is still there.
#[gpui_kit::test]
fn manual_rename_during_generation_wins(cx: &mut TestAppContext) {
    let prompts: Prompts = Default::default();
    let sends: Sends = Default::default();
    let (ws, cx) = mount(cx);
    use_backend(&ws, FakeBackend::manual(&prompts, &sends), cx);
    submit(&ws, cx, "refactor the parser");
    until_prompts(&prompts, 1, cx);
    sends.lock()[0].send(AgentEvent::TextDelta("done".into())).unwrap();
    sends.lock()[0].send(AgentEvent::Done).unwrap();
    until_prompts(&prompts, 2, cx);
    let id = ws.read_with(cx, |w, _| w.chats[w.active].id);
    rename(&ws, id, "Parser work", cx);
    sends.lock()[1].send(AgentEvent::TextDelta("Parser Refactor".into())).unwrap();
    sends.lock()[1].send(AgentEvent::Done).unwrap();
    settle(cx);
    assert!(chat(&ws, cx, |c| c.title == "Parser work"), "rename beats the late title");
    assert!(!chat(&ws, cx, |c| c.title_generated));
}

/// A backend error keeps the placeholder — and the next successful turn
/// gets another shot at a title.
#[gpui_kit::test]
fn backend_error_keeps_the_placeholder(cx: &mut TestAppContext) {
    let prompts: Prompts = Default::default();
    let (ws, cx) = mount(cx);
    use_backend(
        &ws,
        FakeBackend::scripted(
            &prompts,
            vec![
                vec![AgentEvent::TextDelta("reply one".into()), AgentEvent::Done],
                vec![AgentEvent::Error("boom".into())],
                vec![AgentEvent::TextDelta("reply two".into()), AgentEvent::Done],
                vec![AgentEvent::TextDelta("Login Fix".into()), AgentEvent::Done],
            ],
        ),
        cx,
    );
    submit(&ws, cx, "fix the login bug");
    until_prompts(&prompts, 2, cx);
    settle(cx);
    assert!(chat(&ws, cx, |c| c.title == "fix the login bug"), "placeholder kept on error");
    assert!(!chat(&ws, cx, |c| c.title_generated));
    // The flag was never set, so the next completed turn retries.
    submit(&ws, cx, "and the logout bug too");
    until(&ws, cx, title_is("Login Fix"));
    assert_eq!(prompts.lock().len(), 4);
}

/// A chat that already carries a real title — resumed, duplicated, or
/// renamed — is left alone.
#[gpui_kit::test]
fn already_titled_chat_skips_generation(cx: &mut TestAppContext) {
    let prompts: Prompts = Default::default();
    let (ws, cx) = mount(cx);
    use_backend(&ws, FakeBackend::scripted(&prompts, vec![vec![AgentEvent::TextDelta("ok".into()), AgentEvent::Done]]), cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.chats[this.active].title = "Resumed session".into();
        });
    });
    submit(&ws, cx, "continue where we left off");
    until(&ws, cx, |w| !w.chats[w.active].running);
    settle(cx);
    assert_eq!(prompts.lock().len(), 1, "no title turn for a titled chat");
    assert!(chat(&ws, cx, |c| c.title == "Resumed session"));
}
