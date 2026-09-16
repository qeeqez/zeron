//! Headless tests for apply-to-file: the Apply button on a non-shell code
//! block resolves the target (path hint or the project-file picker), writes
//! through the injected `FileWriter`, gates overwrites and read-only
//! threads behind the approval card, and lands the outcome as a note.
//! Narrow imports on purpose (see `composer_testutil`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::dialog::Confirm;
use gpui_kit::component::{IndexPath, Root};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, Focusable, SharedString, TestAppContext, VisualTestContext};

use crate::apply_code::{FileWriter, set_file_writer};
use crate::backend::{AccessMode, ApprovalDecision};
use crate::model::MessageKind;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-apply-test-{}", std::process::id()));
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

/// Records writes and reports a canned set of existing files.
struct FakeWriter {
    existing: Vec<String>,
    writes: parking_lot::Mutex<Vec<(PathBuf, String, String)>>,
}

impl FakeWriter {
    fn install(existing: &[&str]) -> Arc<Self> {
        let fake = Arc::new(Self {
            existing: existing.iter().map(|s| s.to_string()).collect(),
            writes: parking_lot::Mutex::new(Vec::new()),
        });
        set_file_writer(fake.clone());
        fake
    }

    fn writes(&self) -> Vec<(PathBuf, String, String)> {
        self.writes.lock().clone()
    }
}

impl FileWriter for FakeWriter {
    fn exists(&self, _root: &Path, rel: &str) -> bool {
        self.existing.iter().any(|e| e == rel)
    }

    fn write(&self, root: &Path, rel: &str, content: &str) -> Result<(), String> {
        self.writes.lock().push((root.to_path_buf(), rel.to_string(), content.to_string()));
        Ok(())
    }
}

/// A writer whose every write fails.
struct FailWriter;

impl FileWriter for FailWriter {
    fn exists(&self, _root: &Path, _rel: &str) -> bool {
        false
    }

    fn write(&self, _root: &Path, _rel: &str, _content: &str) -> Result<(), String> {
        Err("disk full".into())
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
/// approval poll runs on the test executor, so a single pump isn't enough.
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

/// The last assistant text on the active chat.
fn last_text(ws: &Workspace) -> Option<String> {
    ws.chats[ws.active].messages.iter().rev().find_map(|m| match &m.kind {
        MessageKind::Text(t) => Some(t.to_string()),
        _ => None,
    })
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
    ws.update(cx, |this, cx| this.answer_approval(ix, decision, cx));
}

/// Draw, then click the Apply button on message `ix`'s code block.
fn click_apply(ix: usize, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        let needle = format!("apply-code-{ix}-");
        let apply_id = ids
            .iter()
            .find(|id| format!("{id:?}").contains(&needle))
            .cloned()
            .unwrap_or_else(|| panic!("no {needle}* id in {ids:?}"));
        window.click(apply_id, cx);
    });
    cx.run_until_parked();
}

#[test]
fn apply_writes_hinted_file_and_notes() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeWriter::install(&[]);
    seed(&ws, "```rust\n// path: src/foo.rs\nfn main() {}\n```\n", cx);
    click_apply(0, cx);

    let writes = fake.writes();
    assert_eq!(writes.len(), 1, "writes: {writes:?}");
    let root = ws.read_with(cx, |ws, _| ws.project.root().to_path_buf());
    assert_eq!(writes[0].0, root, "the write lands under the project root");
    assert_eq!(writes[0].1, "src/foo.rs");
    assert!(writes[0].2.starts_with("// path: src/foo.rs"), "code: {:?}", writes[0].2);
    until(&ws, cx, |ws| last_text(ws).is_some_and(|t| t.contains("Wrote `src/foo.rs`")));
}

#[test]
fn apply_without_hint_picks_from_project_files() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeWriter::install(&[]);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.project_files = vec![SharedString::from("docs/guide.md"), SharedString::from("src/main.rs")];
        })
    });
    seed(&ws, "```rust\nfn helper() {}\n```\n", cx);
    click_apply(0, cx);

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("command").visible(), "the apply picker opens");
        // Row 0 is src/main.rs — the .rs match leads the list.
        ws.update(cx, |this, cx| {
            this.apply_palette
                .update(cx, |state, cx| state.set_selected_index(Some(IndexPath::new(0).section(0)), window, cx));
        });
        ws.read(cx)
            .apply_palette
            .read(cx)
            .focus_handle(cx)
            .dispatch_action(&Confirm { secondary: false }, window, cx);
    });
    cx.run_until_parked();

    let writes = fake.writes();
    assert_eq!(writes.len(), 1, "writes: {writes:?}");
    assert_eq!(writes[0].1, "src/main.rs");
    until(&ws, cx, |ws| last_text(ws).is_some_and(|t| t.contains("Wrote `src/main.rs`")));
}

#[test]
fn overwrite_asks_first() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeWriter::install(&["src/existing.rs"]);
    seed(&ws, "```rust\n// path: src/existing.rs\nfn v2() {}\n```\n", cx);
    click_apply(0, cx);

    assert!(fake.writes().is_empty(), "overwrite wrote without asking");
    answer(&ws, cx, ApprovalDecision::Approve);
    until(&ws, cx, |ws| last_text(ws).is_some_and(|t| t.contains("Wrote `src/existing.rs`")));
    let writes = fake.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].2.contains("fn v2()"), "code: {:?}", writes[0].2);
}

#[test]
fn denied_overwrite_never_writes() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeWriter::install(&["src/existing.rs"]);
    seed(&ws, "```rust\n// path: src/existing.rs\nfn v2() {}\n```\n", cx);
    click_apply(0, cx);
    answer(&ws, cx, ApprovalDecision::Deny);
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert!(fake.writes().is_empty(), "denied overwrite wrote");
}

#[test]
fn read_only_mode_confirms_new_files() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeWriter::install(&[]);
    cx.update(|_, cx| ws.update(cx, |this, cx| this.set_access(AccessMode::Supervised, cx)));
    seed(&ws, "```rust\n// path: src/new.rs\nfn main() {}\n```\n", cx);
    click_apply(0, cx);

    assert!(fake.writes().is_empty(), "read-only mode wrote without asking");
    answer(&ws, cx, ApprovalDecision::Approve);
    until(&ws, cx, |ws| last_text(ws).is_some_and(|t| t.contains("Wrote `src/new.rs`")));
    assert_eq!(fake.writes().len(), 1);
}

#[test]
fn always_allow_skips_later_prompts() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let fake = FakeWriter::install(&["src/a.rs"]);
    seed(&ws, "```rust\n// path: src/a.rs\nfn a() {}\n```\n", cx);
    click_apply(0, cx);
    answer(&ws, cx, ApprovalDecision::ApproveForSession);
    until(&ws, cx, |ws| last_text(ws).is_some_and(|t| t.contains("Wrote `src/a.rs`")));
    assert!(ws.read_with(cx, |ws, _| ws.apply_approved));

    // The next overwrite writes directly — no second approval card. The
    // second note is message 3 (note, approval card, "Wrote" note, note).
    seed(&ws, "```rust\n// path: src/a.rs\nfn a2() {}\n```\n", cx);
    click_apply(3, cx);
    assert_eq!(fake.writes().len(), 2, "session-approved apply still prompted");
}

#[test]
fn failed_write_notes_the_error() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    set_file_writer(Arc::new(FailWriter));
    seed(&ws, "```rust\n// path: src/foo.rs\nfn main() {}\n```\n", cx);
    click_apply(0, cx);
    until(&ws, cx, |ws| last_text(ws).is_some_and(|t| t.contains("Apply failed") && t.contains("disk full")));
}

#[test]
fn shell_block_has_no_apply_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    FakeWriter::install(&[]);
    seed(&ws, "```bash\necho hi\n```\n\n```rust\nfn main() {}\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        let apply_count = ids.iter().filter(|id| format!("{id:?}").contains("apply-code-0-")).count();
        assert_eq!(apply_count, 1, "only the non-shell block gets Apply: {ids:?}");
        assert!(has_id_containing(&ids, "run-code-0-"), "the shell block keeps Run: {ids:?}");
    });
}
