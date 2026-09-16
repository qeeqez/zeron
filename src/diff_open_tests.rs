//! Tests for ⌘-click-to-open on diff rows: the per-editor argv builders
//! (`cli_command` / `open_command_at`), the line each row kind resolves to,
//! and the click routing that keeps plain clicks on the review-comment
//! anchor. Imports stay narrow on purpose (see `composer_testutil`).

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    App, AppContext, ElementId, Entity, InputEvent, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, Window,
};

use crate::changes_diff::{DiffLine, DiffLineKind, FileDiff};
use crate::git::{ChangeStatus, FileChange};
use crate::model::ReviewTarget;
use crate::open_in::{ISSUED, PreferredEditor, cli_command, open_command_at, reveal_command};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-diffopen-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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

fn change(path: &str) -> FileChange {
    FileChange {
        path: path.into(),
        source: None,
        status: ChangeStatus::Modified,
        added: 1,
        deleted: 1,
        staged: false,
        diff: None,
        diff_load: 0,
    }
}

/// Hunk + context + removed + added with distinct old/new numbers, so the
/// opened line proves which side was used: context is old 10 / new 20, the
/// removed line is old 11, the added line is new 21.
fn sample_diff() -> FileDiff {
    FileDiff {
        lines: vec![
            DiffLine {
                kind: DiffLineKind::Hunk,
                old: None,
                new: None,
                text: "@@ -10,3 +20,4 @@".into(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                old: Some(10),
                new: Some(20),
                text: "fn main() {".into(),
            },
            DiffLine {
                kind: DiffLineKind::Removed,
                old: Some(11),
                new: None,
                text: "old();".into(),
            },
            DiffLine {
                kind: DiffLineKind::Added,
                old: None,
                new: Some(21),
                text: "new();".into(),
            },
        ],
        truncated: false,
    }
}

/// Seed one expanded change row with `sample_diff` and open the panel.
fn seed_diff(workspace: &Entity<Workspace>, editor: PreferredEditor, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.set_preferred_editor(editor, cx);
            let mut c = change("src/edited.rs");
            c.diff = Some(sample_diff());
            ws.changes = vec![c];
            ws.changes_panel_open = true;
            cx.notify();
        });
    });
}

/// Wait for a background `run_open_command` task to record `n` commands.
fn until_issued(cx: &mut VisualTestContext, n: usize) {
    for _ in 0..200 {
        cx.run_until_parked();
        if ISSUED.lock().len() >= n {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("expected {n} issued commands, got {:?}", ISSUED.lock());
}

/// `window.click` with held modifiers — the test API's click always sends
/// none, so a ⌘-click needs raw down/up events on the target's center.
fn click_with(window: &mut Window, id: impl Into<ElementId>, modifiers: Modifiers, cx: &mut App) {
    window.render_frame(cx);
    let position = window.find(id).bounds().center();
    window.dispatch_event(
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    window.dispatch_event(
        MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn cmd() -> Modifiers {
    Modifiers { platform: true, ..Default::default() }
}

/// The last issued command's `file:line` argument — the CLI form is
/// `[-g] <abs>:<line>` and the `open -a` fallback ends in `--args -g
/// <abs>:<line>`, so the suffix check covers both.
fn issued_target() -> String {
    let cmd = ISSUED.lock()[0].clone();
    cmd.args.last().cloned().unwrap_or_default()
}

#[test]
fn cli_command_uses_each_editors_line_syntax() {
    let cli = std::path::Path::new("/Applications/App.app/Contents/cli");
    let abs = std::path::Path::new("/repo/src/a.rs");
    for editor in [PreferredEditor::VsCode, PreferredEditor::Cursor] {
        let cmd = cli_command(editor, cli, abs, 12).unwrap_or_else(|| panic!("{editor:?} should build a command"));
        assert_eq!(cmd.program, cli.display().to_string());
        assert_eq!(cmd.args, ["-g", "/repo/src/a.rs:12"], "{editor:?} takes -g file:line");
    }
    let cmd = cli_command(PreferredEditor::Zed, cli, abs, 12).expect("zed should build a command");
    assert_eq!(cmd.args, ["/repo/src/a.rs:12"], "Zed takes file:line positionally");
    assert!(cli_command(PreferredEditor::Finder, cli, abs, 12).is_none(), "Finder isn't an editor");
    assert!(cli_command(PreferredEditor::Ask, cli, abs, 12).is_none(), "Ask shows a picker instead");
}

#[test]
fn open_command_at_falls_back_to_open_dash_a() {
    let abs = std::path::Path::new("/repo/src/a.rs");
    // Finder has no line syntax — it reveals the file.
    assert_eq!(open_command_at(PreferredEditor::Finder, abs, 12), Some(reveal_command(abs)));
    // Ask never builds a command — the menu shows the picker instead.
    assert_eq!(open_command_at(PreferredEditor::Ask, abs, 12), None);
    for editor in [PreferredEditor::VsCode, PreferredEditor::Cursor, PreferredEditor::Zed] {
        let cmd = open_command_at(editor, abs, 12).unwrap_or_else(|| panic!("{editor:?} should build a command"));
        if cmd.program == "open" {
            // No bundled CLI at a standard location: `open -a <App>` —
            // VS Code/Cursor still carry `-g file:line` via --args (it lands
            // on a cold launch); Zed's main binary ignores argv entirely.
            if editor == PreferredEditor::Zed {
                assert_eq!(cmd.args, ["-a", "Zed", "/repo/src/a.rs"], "{editor:?}");
            } else {
                assert_eq!(cmd.args[3..], ["-g", "/repo/src/a.rs:12"], "{editor:?}");
            }
        } else if editor == PreferredEditor::Zed {
            assert_eq!(cmd.args, ["/repo/src/a.rs:12"], "{editor:?}");
        } else {
            assert_eq!(cmd.args, ["-g", "/repo/src/a.rs:12"], "{editor:?}");
        }
    }
}

#[gpui_kit::test]
fn open_diff_at_line_resolves_each_row_kind(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, PreferredEditor::VsCode, cx);
    // Context → new side, added → new side, removed → old side, hunk → none.
    for (line_ix, want) in [(1usize, Some(20u32)), (3, Some(21)), (2, Some(11)), (0, None)] {
        ISSUED.lock().clear();
        cx.update(|_, cx| ws.update(cx, |this, cx| this.open_diff_at_line(0, line_ix, cx)));
        match want {
            Some(n) => {
                until_issued(cx, 1);
                assert!(issued_target().ends_with(&format!(":{n}")), "line {line_ix} should open at {n}, got {:?}", ISSUED.lock()[0]);
            },
            None => {
                cx.run_until_parked();
                assert!(ISSUED.lock().is_empty(), "hunk header issues no command");
            },
        }
    }
}

#[gpui_kit::test]
fn ask_editor_falls_back_to_reveal(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, PreferredEditor::Ask, cx);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.open_diff_at_line(0, 1, cx)));
    until_issued(cx, 1);
    let abs = ws.read_with(cx, |w, _| w.project.root().join("src/edited.rs").display().to_string());
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(std::path::Path::new(&abs))]);
}

#[gpui_kit::test]
fn cmd_click_opens_at_line_and_plain_click_comments(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    seed_diff(&ws, PreferredEditor::VsCode, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // ⌘-click the added row → opens at the new-side line, no comment editor.
        click_with(window, ("diff-line", 3usize), cmd(), cx);
    });
    until_issued(cx, 1);
    let abs = ws.read_with(cx, |w, _| w.project.root().join("src/edited.rs").display().to_string());
    assert_eq!(issued_target(), format!("{abs}:21"), "added row opens at new line 21");
    cx.update(|_, cx| assert!(ws.read(cx).review.target.is_none(), "⌘-click doesn't anchor a comment"));

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // Plain click on the same row still anchors the comment editor.
        window.click(("diff-line", 3usize), cx);
        assert_eq!(ws.read(cx).review.target, Some(ReviewTarget { file_ix: 0, line_ix: 3 }), "plain click anchors the comment editor");
    });
}

#[gpui_kit::test]
fn cmd_click_works_on_split_cells(cx: &mut TestAppContext) {
    let (ws, cx) = mount(cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| this.diff_mode = crate::changes_diff::DiffMode::Split);
    });
    seed_diff(&ws, PreferredEditor::VsCode, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The removed line's old cell opens at the old-side number.
        click_with(window, ("diff-cell-old", 2usize), cmd(), cx);
    });
    until_issued(cx, 1);
    assert!(issued_target().ends_with(":11"), "old cell opens at old line 11, got {}", issued_target());

    ISSUED.lock().clear();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The added line's new cell opens at the new-side number.
        click_with(window, ("diff-cell-new", 3usize), cmd(), cx);
    });
    until_issued(cx, 1);
    assert!(issued_target().ends_with(":21"), "new cell opens at new line 21, got {}", issued_target());
}
