//! Headless Snapshots-panel tests: the list rows, expand + cached file
//! list, the restore confirm, and delete. Split from `snapshots_tests.rs`
//! for the SLOC cap — same mount harness, duplicated helpers.

use std::path::PathBuf;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::checkpoints::{TurnCheckpoint, snapshot};
use crate::snapshots::SnapshotFiles;
use crate::workspace::Workspace;

fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-snap-ui-{}", std::process::id()));
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

/// A temp git repo standing in for the chat's workdir — keeps snapshot
/// refs out of the real project repo.
fn temp_repo() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("rixlcode-snap-ui-repo-{}", std::process::id()));
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

/// Drive background-executor collections (snapshot refresh) to landing.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..50 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Give the active chat a workdir + one recorded checkpoint, then open
/// the panel and let the collection land.
fn open_panel_with_snapshot(ws: &Entity<Workspace>, repo: &std::path::Path, cx: &mut VisualTestContext) {
    let store = ws.read_with(cx, |w, _| w.project.dir().join("checkpoints"));
    let checkpoint = snapshot(repo, &store, "c0-0").unwrap();
    ws.update(cx, |this, cx| {
        let chat = &mut this.chats[this.active];
        chat.workdir = repo.to_string_lossy().into_owned();
        chat.checkpoints.push(TurnCheckpoint { ix: 0, at: std::time::SystemTime::now(), checkpoint });
        this.snapshots.open = true;
        this.refresh_snapshots(cx);
    });
    settle(cx);
}

#[test]
fn panel_lists_restores_and_deletes() {
    let Some(repo) = temp_repo() else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_panel_with_snapshot(&ws, &repo, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("snapshot-row", 0usize)).visible(), "checkpoint listed");
    });
    // The "turn" edits the workdir; Restore confirms, then reverts it.
    std::fs::write(repo.join("agent.txt"), "agent").unwrap();
    std::fs::write(repo.join("base.txt"), "agent").unwrap();
    cx.update(|window, cx| {
        window.click(("snapshot-restore", 0usize), cx);
    });
    assert!(cx.has_pending_prompt(), "a dirty restore asks for confirmation");
    cx.simulate_prompt_answer("Restore");
    settle(cx);
    assert!(!repo.join("agent.txt").exists(), "restore removed the new file");
    assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap(), "base");
    // The entry survives a restore — snapshots are a history.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("snapshot-row", 0usize)).visible(), "restore keeps the entry");
        window.click(("snapshot-delete", 0usize), cx);
    });
    settle(cx);
    ws.read_with(cx, |w, _| {
        assert!(w.snapshots.list.is_empty(), "delete dropped the entry");
        assert!(w.chats[0].checkpoints.is_empty(), "checkpoint unpinned from the chat");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find(("snapshot-row", 0usize)).is_none(), "row gone after delete");
    });
    let _ = std::fs::remove_dir_all(&repo);
}

/// Expanding a row lists the paths restore would touch; the list is
/// cached on the row, so a collapse+re-expand after another workdir
/// edit still shows the first computation.
#[test]
fn expand_lists_and_caches_files() {
    let Some(repo) = temp_repo() else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_panel_with_snapshot(&ws, &repo, cx);
    std::fs::write(repo.join("agent.txt"), "agent").unwrap();
    cx.update(|window, cx| {
        window.click(("snapshot-row", 0usize), cx);
    });
    settle(cx);
    ws.read_with(cx, |w, _| {
        let snap = &w.snapshots.list[0];
        assert!(snap.expanded);
        let SnapshotFiles::Loaded(files) = &snap.files else { panic!("files loaded") };
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "agent.txt");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("snapshot-files", 0usize)).visible(), "expanded list renders");
    });
    // Collapse, dirty the workdir again, re-expand — the cached list
    // (not a fresh diff) is what the row shows.
    cx.update(|window, cx| {
        window.click(("snapshot-row", 0usize), cx);
    });
    std::fs::write(repo.join("second.txt"), "second").unwrap();
    cx.update(|window, cx| {
        window.click(("snapshot-row", 0usize), cx);
    });
    settle(cx);
    ws.read_with(cx, |w, _| {
        let SnapshotFiles::Loaded(files) = &w.snapshots.list[0].files else { panic!("still cached") };
        assert_eq!(files.len(), 1, "re-expand reuses the cached list");
    });
    let _ = std::fs::remove_dir_all(&repo);
}

/// A snapshot whose workdir still matches restores on one click — no
/// confirm, no pending prompt.
#[test]
fn restore_skips_confirm_when_clean() {
    let Some(repo) = temp_repo() else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_panel_with_snapshot(&ws, &repo, cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click(("snapshot-restore", 0usize), cx);
    });
    assert!(!cx.has_pending_prompt(), "a clean snapshot restores without asking");
    settle(cx);
    let _ = std::fs::remove_dir_all(&repo);
}

/// The confirm names the count and the first paths; cancelling leaves
/// the workdir alone.
#[test]
fn restore_confirm_names_paths() {
    let Some(repo) = temp_repo() else { return };
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_panel_with_snapshot(&ws, &repo, cx);
    std::fs::write(repo.join("agent.txt"), "agent").unwrap();
    std::fs::write(repo.join("base.txt"), "agent").unwrap();
    cx.update(|window, cx| {
        window.click(("snapshot-restore", 0usize), cx);
    });
    let Some((title, detail)) = cx.pending_prompt() else { panic!("restore asks first") };
    assert_eq!(title, "Restore 2 files?");
    assert!(detail.contains("agent.txt") && detail.contains("base.txt"), "names the paths: {detail}");
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(repo.join("agent.txt").exists(), "cancel keeps the workdir");
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn toggle_opens_and_closes_panel() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new("cmd-shift-s", crate::ToggleSnapshots, Some("workspace"))]);
        window.draw(cx).clear(cx);
        assert!(window.try_find("snapshots-panel").is_none(), "panel starts closed");

        window.press("cmd-shift-s", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("snapshots-panel").visible(), "cmd-shift-s opens the panel");
        assert!(ws.read(cx).snapshots.open);

        window.click("close-snapshots", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("snapshots-panel").is_none(), "close button hides the panel");
        assert!(!ws.read(cx).snapshots.open);
    });
}
