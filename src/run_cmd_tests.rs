//! Tests for run-in-terminal on shell code blocks: the Run button only
//! appears on shell fences, the command runs in the chat's working
//! directory through the injected `CommandRunner`, the result lands as a
//! tool card with the exit code, and ask-mode threads gate the run behind
//! the approval card.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AccessMode, ApprovalDecision};
use crate::dock_badge;
use crate::model::{MessageKind, ToolStatus};
use crate::run_cmd::{CommandOutput, CommandRunner, set_command_runner, shell_for};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-run-test-{}", std::process::id()));
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

/// Records each invocation and replies with a canned output.
struct FakeRunner {
    calls: parking_lot::Mutex<Vec<(String, String, PathBuf)>>,
    out: CommandOutput,
}

impl FakeRunner {
    fn install(out: CommandOutput) -> Arc<Self> {
        let fake = Arc::new(Self { calls: parking_lot::Mutex::new(Vec::new()), out });
        set_command_runner(fake.clone());
        fake
    }

    fn calls(&self) -> Vec<(String, String, PathBuf)> {
        self.calls.lock().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, shell: &str, command: &str, cwd: &std::path::Path) -> CommandOutput {
        self.calls.lock().push((shell.to_string(), command.to_string(), cwd.to_path_buf()));
        self.out.clone()
    }
}

fn seed(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| this.push_note(text.to_string(), cx));
}

/// ElementIds registered by `.test_support()` in the last frame.
fn observed_ids(window: &gpui_kit::Window) -> Vec<ElementId> {
    gpui_kit::base::test_support::snapshots(window)
        .iter()
        .filter_map(|s| s.path().last().cloned())
        .collect()
}

fn has_id_containing(ids: &[ElementId], needle: &str) -> bool {
    ids.iter().any(|id| format!("{id:?}").contains(needle))
}

/// Advance the test clock until `cond` holds or the budget runs out — the
/// run lands on the background executor, so a single pump isn't enough.
fn until(ws: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
    for _ in 0..64 {
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| cond(ws)) {
            return;
        }
    }
    panic!("condition never held");
}

/// The last tool card on the active chat, if any.
fn last_tool(ws: &Workspace) -> Option<(ToolStatus, String, String)> {
    ws.chats[ws.active].messages.iter().rev().find_map(|m| match &m.kind {
        MessageKind::Tool(t) => Some((t.status, t.detail.to_string(), t.output.to_string())),
        _ => None,
    })
}

/// Put the active thread in read-only mode and kick off a run — leaves the
/// approval card pending.
fn request_run(ws: &Entity<Workspace>, cx: &mut VisualTestContext, command: &str) {
    cx.update(|_, cx| ws.update(cx, |this, cx| this.set_access(AccessMode::Supervised, cx)));
    cx.update(|_, cx| ws.update(cx, |this, cx| this.run_command_block(command.to_string(), "sh", cx)));
    cx.run_until_parked();
}

/// Answer the pending approval card with `decision`.
fn answer(ws: &Entity<Workspace>, cx: &mut VisualTestContext, decision: ApprovalDecision) {
    let ix = ws.read_with(cx, |ws, _| {
        ws.chats[ws.active]
            .messages
            .iter()
            .position(|m| matches!(&m.kind, MessageKind::Approval(_)))
            .unwrap()
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.answer_approval(ix, decision, cx)));
}

#[test]
fn shell_for_maps_shell_tags_only() {
    assert_eq!(shell_for("sh"), Some("sh"));
    assert_eq!(shell_for("shell"), Some("sh"));
    assert_eq!(shell_for("Bash"), Some("bash"));
    assert_eq!(shell_for("zsh"), Some("zsh"));
    for lang in ["rust", "python", "console", ""] {
        assert_eq!(shell_for(lang), None, "{lang} should not be runnable");
    }
}

#[test]
fn run_shell_block_runs_in_project_dir_and_lands_card() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeRunner::install(CommandOutput {
        stdout: "hello\n".into(),
        stderr: String::new(),
        code: Some(0),
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.run_command_block("echo hello".to_string(), "sh", cx)));
    until(&ws, cx, |ws| last_tool(ws).is_some_and(|(s, _, _)| s != ToolStatus::Running));

    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "runner calls: {calls:?}");
    let (shell, command, cwd) = &calls[0];
    assert_eq!(shell, "sh");
    assert_eq!(command, "echo hello");
    let root = ws.read_with(cx, |ws, _| ws.project.root().to_path_buf());
    assert_eq!(cwd, &root, "command ran in {cwd:?}, not the project root {root:?}");

    let (status, detail, output) = ws.read_with(cx, |ws, _| last_tool(ws)).unwrap();
    assert_eq!(status, ToolStatus::Done);
    assert_eq!(detail, "echo hello");
    assert!(output.contains("hello"), "output: {output:?}");
    assert!(output.contains("[exit 0]"), "output: {output:?}");
}

#[test]
fn nonzero_exit_marks_card_failed() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    FakeRunner::install(CommandOutput {
        stdout: String::new(),
        stderr: "boom\n".into(),
        code: Some(3),
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.run_command_block("exit 3".to_string(), "bash", cx)));
    until(&ws, cx, |ws| last_tool(ws).is_some_and(|(s, _, _)| s != ToolStatus::Running));

    let (status, _, output) = ws.read_with(cx, |ws, _| last_tool(ws)).unwrap();
    assert_eq!(status, ToolStatus::Failed);
    assert!(output.contains("boom"), "output: {output:?}");
    assert!(output.contains("[exit 3]"), "output: {output:?}");
}

#[test]
fn background_run_marks_chat_unread_and_badges_dock() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    FakeRunner::install(CommandOutput {
        stdout: "done\n".into(),
        stderr: String::new(),
        code: Some(0),
    });
    cx.update(|_, cx| ws.update(cx, |this, cx| this.run_command_block("echo hi".to_string(), "sh", cx)));
    // Switch away before the run lands — a finished run on a background
    // chat flags it unread, and the dock badge mirrors that count.
    cx.update(|_, cx| ws.update(cx, |this, cx| this.new_chat(cx)));
    until(&ws, cx, |ws| ws.chats[0].unread);
    cx.run_until_parked();
    assert_eq!(dock_badge::last_badge(), Some(1), "a background run's unread chat badges the dock");
}

#[test]
fn read_only_mode_asks_before_running() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeRunner::install(CommandOutput { stdout: "ok\n".into(), stderr: String::new(), code: Some(0) });
    request_run(&ws, cx, "rm -rf x");

    // The approval card is up; nothing ran yet.
    assert!(fake.calls().is_empty(), "command ran without approval");
    let pending = ws.read_with(cx, |ws, _| {
        ws.chats[ws.active]
            .messages
            .iter()
            .any(|m| matches!(&m.kind, MessageKind::Approval(a) if a.decision.is_none()))
    });
    assert!(pending, "no pending approval card");

    // Approve it — the command runs and the card lands.
    answer(&ws, cx, ApprovalDecision::Approve);
    until(&ws, cx, |ws| last_tool(ws).is_some_and(|(s, _, _)| s != ToolStatus::Running));
    assert_eq!(fake.calls().len(), 1, "approved command never ran");
    let (status, _, _) = ws.read_with(cx, |ws, _| last_tool(ws)).unwrap();
    assert_eq!(status, ToolStatus::Done);
}

#[test]
fn denied_command_never_runs() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeRunner::install(CommandOutput { stdout: String::new(), stderr: String::new(), code: Some(0) });
    request_run(&ws, cx, "rm -rf x");
    answer(&ws, cx, ApprovalDecision::Deny);
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert!(fake.calls().is_empty(), "denied command ran");
    assert!(ws.read_with(cx, |ws, _| last_tool(ws)).is_none(), "denied command left a tool card");
}

#[test]
fn always_allow_runs_later_commands_without_asking() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeRunner::install(CommandOutput { stdout: "ok\n".into(), stderr: String::new(), code: Some(0) });
    request_run(&ws, cx, "first");
    answer(&ws, cx, ApprovalDecision::ApproveForSession);
    until(&ws, cx, |ws| last_tool(ws).is_some_and(|(s, _, _)| s != ToolStatus::Running));

    // The next run skips the prompt entirely.
    cx.update(|_, cx| ws.update(cx, |this, cx| this.run_command_block("second".to_string(), "sh", cx)));
    until(&ws, cx, |ws| ws.chats[ws.active].messages.iter().filter(|m| matches!(&m.kind, MessageKind::Tool(_))).count() == 2);
    assert_eq!(fake.calls().len(), 2, "calls: {:?}", fake.calls());
    let approvals =
        ws.read_with(cx, |ws, _| ws.chats[ws.active].messages.iter().filter(|m| matches!(&m.kind, MessageKind::Approval(_))).count());
    assert_eq!(approvals, 1, "a second approval prompt appeared");
}

#[test]
fn shell_block_shows_run_button_and_output_card() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    FakeRunner::install(CommandOutput { stdout: "hi\n".into(), stderr: String::new(), code: Some(0) });
    seed(&ws, "Run it:\n\n```bash\necho hi\n```\n\n```rust\nfn main() {}\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "run-code-0-"), "Run button missing on bash block: {ids:?}");
        let run_id = ids.iter().find(|id| format!("{id:?}").contains("run-code-0-")).cloned().unwrap();
        window.click(run_id, cx);
    });
    until(&ws, cx, |ws| last_tool(ws).is_some_and(|(s, _, _)| s != ToolStatus::Running));

    let (status, detail, output) = ws.read_with(cx, |ws, _| last_tool(ws)).unwrap();
    assert_eq!(status, ToolStatus::Done);
    assert_eq!(detail, "echo hi");
    assert!(output.contains("[exit 0]"), "output: {output:?}");
    // The card is a transcript message — the scroller grew to show it.
    let count = ws.read_with(cx, |ws, _| ws.chats[ws.active].messages.len());
    assert_eq!(count, 2, "expected the note + the output card, got {count} messages");
}

#[test]
fn non_shell_block_has_no_run_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    seed(&ws, "```rust\nfn main() {}\n```\n\n```\nplain\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(!has_id_containing(&ids, "run-code-"), "Run button on a non-shell block: {ids:?}");
        assert!(has_id_containing(&ids, "copy-code-0-"), "copy button missing: {ids:?}");
    });
}
