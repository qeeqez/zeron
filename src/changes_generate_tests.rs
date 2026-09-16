//! Tests for AI commit-message generation: a scripted fake backend stands
//! in for the agent, real git provides the staged diff. Same
//! `TestAppContext::single()` pattern as `changes_git_ui_tests.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::git::BranchStatus;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-generate-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    cx.add_window_view(Workspace::new)
}

/// A temp git repo with one commit — the ops need a HEAD. Returns None when
/// git isn't installed.
fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-generate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if !run(&dir, &["init", "-q"]) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    assert!(run(&dir, &["config", "user.email", "t@t"]));
    assert!(run(&dir, &["config", "user.name", "t"]));
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(run(&dir, &["commit", "-qm", "init"]));
    Some(dir)
}

fn run(dir: &std::path::Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Stage a change in `dir` so `git diff --cached` has content.
fn stage_edit(dir: &std::path::Path) {
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    assert!(run(dir, &["add", "f.txt"]));
}

/// A backend that replays a scripted event stream and records the prompt it
/// was sent — stands in for the agent without spawning a process.
struct FakeBackend {
    events: Vec<AgentEvent>,
    prompts: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        self.prompts.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        for e in &self.events {
            let _ = tx.send(e.clone());
        }
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Point the workspace at `backend` with a model selected — generation
/// refuses to run without one.
fn use_backend(ws: &Entity<Workspace>, backend: impl AgentBackend + 'static, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.backend = std::sync::Arc::new(backend);
            this.model = "m".into();
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

fn commit_value(ws: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    ws.read_with(cx, |w, app| w.git.commit_input.read(app).value().to_string())
}

fn note(ws: &Entity<Workspace>, cx: &VisualTestContext) -> Option<(String, bool)> {
    ws.read_with(cx, |w, _| w.git.note.clone())
}

/// A staged diff plus a scripted reply fills the commit box with the
/// conventional-commit subject — markdown decoration is stripped — and the
/// prompt carried the diff to the backend.
#[test]
fn generate_fills_the_commit_box() {
    let Some(dir) = temp_repo("fill") else { return };
    stage_edit(&dir);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    use_backend(
        &ws,
        FakeBackend {
            events: vec![
                AgentEvent::TextDelta("`feat(api): add retry`\n\nNotes the user shouldn't see.".into()),
                AgentEvent::Done,
            ],
            prompts: prompts.clone(),
        },
        cx,
    );
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.generate_commit_message(cx);
            assert!(this.git.generating, "generation started");
        });
    });
    until(&ws, cx, |w| !w.git.generating);
    assert_eq!(commit_value(&ws, cx), "feat(api): add retry", "subject landed in the box");
    assert!(note(&ws, cx).is_none(), "a successful fill leaves no note");
    let sent = prompts.lock().clone();
    assert_eq!(sent.len(), 1, "one turn was sent");
    assert!(sent[0].contains("+two"), "prompt carried the staged diff");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The filled box stays editable — the user reviews the generated message
/// before committing.
#[test]
fn generated_message_stays_editable() {
    let Some(dir) = temp_repo("edit") else { return };
    stage_edit(&dir);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    use_backend(
        &ws,
        FakeBackend {
            events: vec![AgentEvent::TextDelta("fix: handle empty input".into()), AgentEvent::Done],
            prompts: std::sync::Arc::new(parking_lot::Mutex::new(Vec::new())),
        },
        cx,
    );
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.generate_commit_message(cx);
        });
    });
    until(&ws, cx, |w| !w.git.generating);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.git.commit_input.update(cx, |s, cx| s.set_value("fix: better wording", window, cx));
        });
    });
    assert_eq!(commit_value(&ws, cx), "fix: better wording", "the box takes edits after a fill");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nothing staged → a note, no turn sent, the box untouched.
#[test]
fn empty_staged_diff_leaves_a_note() {
    let Some(dir) = temp_repo("empty") else { return };
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    use_backend(&ws, FakeBackend { events: vec![AgentEvent::Done], prompts: prompts.clone() }, cx);
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.generate_commit_message(cx);
        });
    });
    until(&ws, cx, |w| !w.git.generating);
    let (text, is_error) = note(&ws, cx).expect("a note landed");
    assert!(!is_error, "an empty diff is guidance, not a failure");
    assert!(text.contains("staged"), "note explains: {text}");
    assert!(commit_value(&ws, cx).is_empty(), "the box stays empty");
    assert!(prompts.lock().is_empty(), "no turn was sent");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A backend error lands as an error note; the box stays untouched.
#[test]
fn backend_error_leaves_a_note() {
    let Some(dir) = temp_repo("error") else { return };
    stage_edit(&dir);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    use_backend(
        &ws,
        FakeBackend {
            events: vec![AgentEvent::Error("rate limit exceeded".into()), AgentEvent::Done],
            prompts: std::sync::Arc::new(parking_lot::Mutex::new(Vec::new())),
        },
        cx,
    );
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.generate_commit_message(cx);
        });
    });
    until(&ws, cx, |w| !w.git.generating);
    assert_eq!(note(&ws, cx), Some(("rate limit exceeded".to_string(), true)), "the error surfaced as a note");
    assert!(commit_value(&ws, cx).is_empty(), "the box stays empty");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Clicking the ✦ button runs a generation and fills the box — the spinner
/// shows while the turn is in flight.
#[test]
fn generate_button_fills_the_box() {
    let Some(dir) = temp_repo("button") else { return };
    stage_edit(&dir);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    use_backend(
        &ws,
        FakeBackend {
            events: vec![AgentEvent::TextDelta("feat: add the thing".into()), AgentEvent::Done],
            prompts: std::sync::Arc::new(parking_lot::Mutex::new(Vec::new())),
        },
        cx,
    );
    cx.update(|_window, cx| {
        ws.update(cx, |this, cx| {
            this.project = crate::project::Project::open(&dir);
            this.git.branch = Some(BranchStatus { name: "main".into(), upstream: None, ahead: 0, behind: 0 });
            this.changes_panel_open = true;
            cx.notify();
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("generate-message").visible(), "the ✦ button renders");
        window.click("generate-message", cx);
    });
    assert!(ws.read_with(cx, |w, _| w.git.generating), "the click started a generation");
    until(&ws, cx, |w| !w.git.generating);
    assert_eq!(commit_value(&ws, cx), "feat: add the thing", "the click filled the box");
    let _ = std::fs::remove_dir_all(&dir);
}
