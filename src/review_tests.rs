//! Tests for diff review: clicking a diff line opens the anchored comment
//! editor, comments collect/edit/remove, and Send review ships a structured
//! `path:line` message through the normal send path (via a recording fake —
//! no real subprocess).
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::changes_diff::{DiffLine, DiffLineKind, FileDiff};
use crate::git::{ChangeStatus, FileChange};
use crate::model::{MessageKind, ReviewComment, ReviewTarget};
use crate::workspace::Workspace;

/// A backend that records each prompt instead of spawning — the send-review
/// assertion without a real subprocess.
struct PromptBackend {
    prompts: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for PromptBackend {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        self.prompts.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-review-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
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

/// Point the workspace at the recording backend with a model set, so
/// `start_reply` dispatches to `backend.send` instead of an error note.
fn use_prompt_backend(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> std::sync::Arc<parking_lot::Mutex<Vec<String>>> {
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(PromptBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    prompts
}

fn change(path: &str, status: ChangeStatus, added: u32, deleted: u32) -> FileChange {
    FileChange {
        path: path.into(),
        source: None,
        status,
        added,
        deleted,
        diff: None,
        diff_load: 0,
    }
}

/// Hunk + context + removed + added — the added line is `new = 2`, the
/// removed line is `old = 2`, the context line is `1` on both sides.
fn sample_diff() -> FileDiff {
    FileDiff {
        lines: vec![
            DiffLine {
                kind: DiffLineKind::Hunk,
                old: None,
                new: None,
                text: "@@ -1,3 +1,4 @@".into(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                old: Some(1),
                new: Some(1),
                text: "fn main() {".into(),
            },
            DiffLine {
                kind: DiffLineKind::Removed,
                old: Some(2),
                new: None,
                text: "old();".into(),
            },
            DiffLine {
                kind: DiffLineKind::Added,
                old: None,
                new: Some(2),
                text: "new();".into(),
            },
        ],
        truncated: false,
    }
}

/// Seed one expanded change row with `sample_diff` and open the panel.
fn seed_diff(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            let mut c = change("src/edited.rs", ChangeStatus::Modified, 1, 1);
            c.diff = Some(sample_diff());
            ws.changes = vec![c];
            ws.changes_panel_open = true;
            cx.notify();
        });
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

#[gpui_kit::test]
fn clicking_diff_line_opens_anchored_comment_editor(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("review-editor").is_none(), "editor starts closed");

        // The context line (old 1 / new 1) is commentable.
        window.click(("diff-line", 1usize), cx);
        assert_eq!(ws.read(cx).review.target, Some(ReviewTarget { file_ix: 0, line_ix: 1 }), "click anchors the editor to file 0, line 1");
        window.draw(cx).clear(cx);
        assert!(window.find("review-editor").visible(), "editor renders under the line");

        // The hunk header carries no line number — clicking it is ignored.
        window.click(("diff-line", 0usize), cx);
        assert_eq!(ws.read(cx).review.target, Some(ReviewTarget { file_ix: 0, line_ix: 1 }), "hunk click ignored");

        // Clicking the anchored line again toggles the editor closed.
        window.click(("diff-line", 1usize), cx);
        assert!(ws.read(cx).review.target.is_none(), "second click closes the editor");
    });
}

#[gpui_kit::test]
fn comments_collect_edit_and_remove(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            // Comment on the context line → src/edited.rs:1.
            this.open_review_comment(0, 1, window, cx);
            this.review.input.update(cx, |s, cx| s.set_value("fix this", window, cx));
            this.commit_review_comment(cx);
            assert_eq!(
                this.review.comments,
                vec![ReviewComment {
                    path: "src/edited.rs".into(),
                    line: 1,
                    old_side: false,
                    code: "fn main() {".into(),
                    text: "fix this".into(),
                }],
                "commit collects the anchored comment"
            );

            // Comment on the added line → new-side number 2.
            this.open_review_comment(0, 3, window, cx);
            this.review.input.update(cx, |s, cx| s.set_value("and this", window, cx));
            this.commit_review_comment(cx);
            assert_eq!(this.review.comments.len(), 2);
            assert_eq!(this.review.comments[1].line, 2, "added line uses the new-side number");

            // Reopening the first line seeds the editor for an edit.
            this.open_review_comment(0, 1, window, cx);
            assert_eq!(this.review.input.read(cx).value(), "fix this", "editor is seeded with the existing comment");
            this.review.input.update(cx, |s, cx| s.set_value("updated", window, cx));
            this.commit_review_comment(cx);
            assert_eq!(this.review.comments.len(), 2, "edit replaces, not appends");
            assert_eq!(this.review.comments[0].text, "updated");

            this.remove_review_comment(0, cx);
            assert_eq!(this.review.comments.len(), 1);
            assert_eq!(this.review.comments[0].text, "and this");
        });
    });
}

#[gpui_kit::test]
fn send_review_formats_file_line_and_comment(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    let prompts = use_prompt_backend(&ws, cx);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.open_review_comment(0, 2, window, cx); // removed line → old 2
            this.review.input.update(cx, |s, cx| s.set_value("why remove this?", window, cx));
            this.commit_review_comment(cx);
            this.open_review_comment(0, 3, window, cx); // added line → new 2
            this.review.input.update(cx, |s, cx| s.set_value("rename suggested", window, cx));
            this.commit_review_comment(cx);

            this.send_review(window, cx);
            assert!(this.review.comments.is_empty(), "send clears the pending review");
            assert!(this.review.target.is_none());
        });
    });
    let sent = prompts.lock().clone();
    assert_eq!(sent.len(), 1, "send review produces exactly one backend turn");
    assert!(sent[0].contains("Review comments"), "structured header: {}", sent[0]);
    // Both sides of line 2 survive — the removed-line comment is tagged.
    assert!(sent[0].contains("src/edited.rs:2 (removed line)"), "removed-side entry: {}", sent[0]);
    assert!(sent[0].contains("why remove this?"), "comment text: {}", sent[0]);
    assert!(sent[0].contains("rename suggested"), "second comment: {}", sent[0]);
    ws.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        assert!(msgs.iter().any(|t| t.contains("src/edited.rs:2")), "review appears as a user message");
    });
}

#[gpui_kit::test]
fn empty_review_does_not_send(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    let prompts = use_prompt_backend(&ws, cx);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| this.send_review(window, cx));
    });
    assert!(prompts.lock().is_empty(), "empty review must not reach the backend");
    ws.read_with(cx, |ws, _| {
        assert!(texts(ws).is_empty(), "empty review adds no user message");
    });
}

/// The full headless path: click a diff line → type in the comment box →
/// Enter commits → the banner counts the pending review → Send review ships
/// the structured message to the backend.
#[gpui_kit::test]
fn click_type_send_review_end_to_end(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    let prompts = use_prompt_backend(&ws, cx);
    seed_diff(&ws, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("diff-line", 3usize), cx); // added line, new = 2
        window.draw(cx).clear(cx);
        assert!(window.find("review-editor").visible(), "click opens the comment box");
    });
    // Focus lands on the next frame — the deferred focus must run before
    // typing reaches the input.
    cx.update(|window, cx| {
        window.input("rename this call", cx);
        window.press("enter", cx);
    });
    // The PressEnter subscription commits on the next effect flush.
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert_eq!(ws.read(cx).review.comments.len(), 1, "Enter commits the comment");
        assert!(ws.read(cx).review.target.is_none(), "commit closes the editor");
        window.draw(cx).clear(cx);
        assert!(window.find("review-banner").visible(), "banner shows the pending review");

        window.click("send-review", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("review-banner").is_none(), "send clears the banner");
    });
    let sent = prompts.lock().clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("src/edited.rs:2"), "structured file:line: {}", sent[0]);
    assert!(sent[0].contains("rename this call"), "comment body: {}", sent[0]);
}
