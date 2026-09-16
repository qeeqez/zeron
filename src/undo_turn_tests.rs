//! Headless "Undo turn" tests: the hover button on the last user message
//! restores the workdir to the turn's checkpoint, truncates the turn and
//! seeds the composer; a dirty restore confirms first (naming the count),
//! a clean one doesn't; and the affordance hides while a reply runs or no
//! checkpoint exists. Split from `checkpoints_tests.rs` for the SLOC cap —
//! same mount harness.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use std::path::PathBuf;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
use crate::checkpoints::{Checkpoint, TurnCheckpoint};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// A backend that edits the turn's workdir like a real agent would —
/// the files it writes are what "Undo turn" must restore.
struct WriteBackend;

impl AgentBackend for WriteBackend {
    fn name(&self) -> &'static str {
        "write"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, ctx: &crate::backend::TurnContext) -> ReplyStream {
        std::fs::write(ctx.cwd.join("agent.txt"), "agent").unwrap();
        std::fs::write(ctx.cwd.join("base.txt"), "agent").unwrap();
        quiet_stream()
    }
}

/// A backend that replies without touching the workdir — its checkpoint
/// diffs clean, so "Undo turn" skips the confirm.
struct QuietBackend;

impl AgentBackend for QuietBackend {
    fn name(&self) -> &'static str {
        "quiet"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, _ctx: &crate::backend::TurnContext) -> ReplyStream {
        quiet_stream()
    }
}

/// A finished reply stream: one text delta, then Done.
fn quiet_stream() -> ReplyStream {
    let (tx, events) = std::sync::mpsc::channel();
    let _ = tx.send(AgentEvent::TextDelta("done".into()));
    let _ = tx.send(AgentEvent::Done);
    ReplyStream {
        events,
        child: None,
        cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-undo-ui-{}", std::process::id()));
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

/// A temp git repo the chat's turn runs in — keeps checkpoint refs out of
/// the real project repo.
fn temp_repo() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-undo-repo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !git(&["init", "-q"]) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    std::fs::write(dir.join("base.txt"), "base").unwrap();
    assert!(git(&["add", "."]));
    assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
    Some(dir)
}

/// A plain temp dir — turns snapshot it via the file-copy fallback.
fn temp_workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-undo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Send `text` on `backend` in `workdir` and wait out the turn — the pump
/// thread is real, so poll with real sleeps, not just clock advances.
fn send_turn(
    ws: &Entity<Workspace>, cx: &mut VisualTestContext, backend: std::sync::Arc<dyn AgentBackend>, workdir: &std::path::Path, text: &str,
) {
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = backend;
            this.chats[this.active].workdir = workdir.to_string_lossy().into_owned();
            this.composer.update(cx, |composer, cx| composer.set_value(text, window, cx));
            this.send(window, cx);
        });
    });
    for _ in 0..200 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| !ws.chats[0].running) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    ws.read_with(cx, |ws, _| assert!(!ws.chats[0].running, "turn finished"));
}

/// Push a message without starting a reply; returns its timestamp for
/// pinning a `TurnCheckpoint`.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, role: Role, text: &str) -> std::time::SystemTime {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            alternatives: vec![],
            role,
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
    ws.read_with(cx, |ws, _| ws.chats[ws.active].messages.last().unwrap().at)
}

/// Pin a checkpoint entry to message `ix` — the storage path needn't
/// exist for affordance tests, only the entry does.
fn pin_checkpoint(ws: &Entity<Workspace>, cx: &mut VisualTestContext, ix: usize, at: std::time::SystemTime) {
    ws.update(cx, |this, _| {
        this.chats[this.active].checkpoints.push(TurnCheckpoint {
            ix,
            at,
            checkpoint: Checkpoint::Copy(PathBuf::from("/definitely/gone")),
        });
    });
}

/// The composer text.
fn draft(ws: &Entity<Workspace>, cx: &VisualTestContext) -> String {
    ws.read_with(cx, |ws, app| ws.composer.read(app).value().to_string())
}

#[test]
fn undo_restores_files_truncates_and_seeds_draft() {
    let Some(repo) = temp_repo() else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    send_turn(&ws, cx, std::sync::Arc::new(WriteBackend), &repo, "hi");
    ws.read_with(cx, |ws, _| {
        let chat = &ws.chats[0];
        assert_eq!(chat.checkpoints.len(), 1, "turn recorded a checkpoint");
        assert_eq!(chat.messages.len(), 2, "user message + reply");
    });
    assert!(repo.join("agent.txt").exists(), "backend wrote during the turn");

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 0usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("undo", 0usize)).visible(), "Undo turn reveals on hover");
        window.click(("undo", 0usize), cx);
    });
    // The backend dirtied the workdir — the undo confirms first.
    assert!(cx.has_pending_prompt(), "a dirty restore asks for confirmation");
    cx.simulate_prompt_answer("Undo turn");
    cx.run_until_parked();
    assert!(!repo.join("agent.txt").exists(), "undo removed the turn's new file");
    assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap(), "base");
    ws.read_with(cx, |ws, _| {
        let chat = &ws.chats[0];
        assert!(chat.messages.is_empty(), "undo dropped the turn's messages");
        assert!(chat.checkpoints.is_empty(), "consumed checkpoint pruned");
    });
    assert_eq!(draft(&ws, cx), "hi", "the undone message lands back in the composer");
    let _ = std::fs::remove_dir_all(&repo);
}

/// A turn that left the workdir untouched undoes on one click — no
/// confirm, no pending prompt.
#[test]
fn clean_turn_skips_confirm() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("clean");
    send_turn(&ws, cx, std::sync::Arc::new(QuietBackend), &workdir, "hi");
    ws.read_with(cx, |ws, _| assert_eq!(ws.chats[0].checkpoints.len(), 1, "turn recorded a checkpoint"));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.undo_turn(0, window, cx));
    });
    assert!(!cx.has_pending_prompt(), "a clean diff undoes without asking");
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.chats[0].messages.is_empty(), "turn truncated"));
    assert_eq!(draft(&ws, cx), "hi");
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The confirm names the file count and the first paths; cancelling keeps
/// the workdir and the transcript.
#[test]
fn undo_confirm_names_paths_and_cancels() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let workdir = temp_workdir("cancel");
    std::fs::write(workdir.join("base.txt"), "base").unwrap();
    send_turn(&ws, cx, std::sync::Arc::new(WriteBackend), &workdir, "hi");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.undo_turn(0, window, cx));
    });
    let Some((title, detail)) = cx.pending_prompt() else { panic!("undo asks first") };
    assert_eq!(title, "Undo turn and restore 2 files?");
    assert!(detail.contains("agent.txt") && detail.contains("base.txt"), "names the paths: {detail}");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(workdir.join("agent.txt").exists(), "cancel keeps the workdir");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.chats[0].messages.len(), 2, "cancel keeps the transcript");
        assert_eq!(ws.chats[0].checkpoints.len(), 1, "checkpoint survives a cancel");
    });
    assert_eq!(draft(&ws, cx), "", "cancel leaves the composer alone");
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The affordance hides while a reply runs, on a message with no
/// checkpoint (a loaded old chat), and on any user message that isn't the
/// last — undoing there would drop later turns too.
#[test]
fn undo_button_only_on_checkpointed_last_user_message() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let at0 = push(&ws, cx, Role::User, "first");
    push(&ws, cx, Role::Assistant, "reply one");
    let at2 = push(&ws, cx, Role::User, "second");
    pin_checkpoint(&ws, cx, 0, at0);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("undo", 0usize)).is_none(), "no undo on an older turn");
        assert!(window.try_find(("undo", 2usize)).is_none(), "no undo without a checkpoint");
    });
    pin_checkpoint(&ws, cx, 2, at2);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.hover(("msg", 2usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("undo", 2usize)).visible(), "undo shows on the checkpointed last user message");
    });
    ws.update(cx, |this, _| this.chats[this.active].running = true);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("undo", 2usize)).is_none(), "no undo while a reply runs");
    });
}
