//! Headless tests for the "Archive read chats" sweep: the palette command
//! hides at zero, the confirm names the count, and only qualifying chats
//! (read, idle, unpinned, off screen) archive — unread, running, pinned,
//! already-archived and the active chat survive. Mount pattern matches
//! `select_tests.rs`.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-sweep-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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

/// `n` extra chats on top of the one the workspace opens with — returns
/// every chat id in `chats` order.
fn chats(ws: &Entity<Workspace>, n: usize, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            for _ in 0..n {
                this.new_chat(cx);
            }
            this.chats.iter().map(|c| c.id).collect()
        })
    })
}

/// In-app toasts mounted under the Root's notification layer. Goes through
/// `app` (not the `VisualTestContext`) so it can run after prompt answers —
/// `cx` borrows `app` for its whole live range.
fn toast_count(app: &TestAppContext) -> usize {
    let Some(&window) = app.windows().first() else { return 0 };
    app.update(|cx| {
        window
            .update(cx, |_view, window, cx| {
                let Some(Some(root)) = window.root::<Root>() else { return 0 };
                root.read(cx).notification.read(cx).notifications().len()
            })
            .unwrap_or(0)
    })
}

fn archived(ws: &Entity<Workspace>, id: u64, app: &TestAppContext) -> bool {
    app.read(|cx| ws.read(cx).chats.iter().find(|c| c.id == id).unwrap().archived)
}

/// The sweep archives every qualifying chat and leaves unread, running,
/// pinned, already-archived and active chats alone.
#[test]
fn sweep_archives_only_read_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // chats[0] read+qualifying, [1] unread, [2] running, [3] pinned,
    // [4] already archived, [5] read+qualifying, [6] active (newest).
    let ids = chats(&ws, 6, cx);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.chats[1].unread = true;
            this.chats[2].running = true;
            this.chats[3].pinned = true;
            this.chats[4].archived = true;
        })
    });
    cx.update(|window, cx| ws.update(cx, |this, cx| this.archive_read_chats(window, cx)));
    let Some((title, detail)) = app.pending_prompt() else { panic!("the sweep asks for confirmation") };
    assert_eq!(title, "Archive 2 chat(s)?", "the confirm names the qualifying count");
    assert!(detail.contains("Unread"), "the detail names what survives: {detail}");
    app.simulate_prompt_answer("Archive");
    app.run_until_parked();
    assert!(archived(&ws, ids[0], &app), "read chat archived");
    assert!(!archived(&ws, ids[1], &app), "unread survives");
    assert!(!archived(&ws, ids[2], &app), "running survives");
    assert!(!archived(&ws, ids[3], &app), "pinned survives");
    assert!(archived(&ws, ids[4], &app), "was already archived");
    assert!(archived(&ws, ids[5], &app), "second read chat archived");
    assert!(!archived(&ws, ids[6], &app), "the active chat survives");
    assert_eq!(toast_count(&app), 1, "the sweep reports with a toast");
}

/// Cancelling the confirm leaves every chat in place.
#[test]
fn sweep_cancel_keeps_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| ws.update(cx, |this, cx| this.archive_read_chats(window, cx)));
    assert!(app.has_pending_prompt());
    app.simulate_prompt_answer("Cancel");
    app.run_until_parked();
    assert!(!archived(&ws, ids[0], &app), "cancel keeps the first chat live");
    assert!(!archived(&ws, ids[1], &app), "cancel keeps the second chat live");
    assert_eq!(toast_count(&app), 0, "a cancelled sweep posts no toast");
}

/// With nothing qualifying the sweep never prompts — the palette hides
/// the command, and a stale call still no-ops.
#[test]
fn sweep_no_qualifying_chats_noops() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // One chat, active — nothing qualifies.
    cx.update(|window, cx| ws.update(cx, |this, cx| this.archive_read_chats(window, cx)));
    assert!(!app.has_pending_prompt(), "zero qualifying chats never prompts");
    assert_eq!(toast_count(&app), 0, "no toast either");
}

/// The palette's "Archive Read Chats" row exists only while a chat
/// qualifies — same gate as "Stop All Replies".
#[test]
fn palette_hides_sweep_at_zero() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let has_sweep = |chats: &[crate::palette_items::ChatSnapshot]| {
        crate::palette_items::build_entries(chats, "archive read", 0)
            .iter()
            .any(|e| matches!(e, crate::palette_items::Entry::Command(spec) if spec.label == "Archive Read Chats"))
    };
    // Only the active chat — nothing qualifies.
    let snaps = ws.read_with(cx, |ws, _| ws.palette_chats());
    assert!(!has_sweep(&snaps), "the command hides when no chat qualifies");
    // A second read chat qualifies.
    let ids = chats(&ws, 1, cx);
    let snaps = ws.read_with(cx, |ws, _| ws.palette_chats());
    assert!(has_sweep(&snaps), "a qualifying chat lists the command");
    // Flag it unread — the command hides again.
    let id = ids[0];
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let ix = this.chats.iter().position(|c| c.id == id).unwrap();
            this.chats[ix].unread = true;
        })
    });
    let snaps = ws.read_with(cx, |ws, _| ws.palette_chats());
    assert!(!has_sweep(&snaps), "an unread-only list hides the command");
}

/// Confirming the palette row runs the sweep — the `Run` effect resolves
/// through `entry_at` like every other command.
#[test]
fn palette_sweep_confirms() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 1, cx);
    let snaps = ws.read_with(cx, |ws, _| ws.palette_chats());
    let entries = crate::palette_items::build_entries(&snaps, "archive read", 0);
    let row = entries
        .iter()
        .position(|e| matches!(e, crate::palette_items::Entry::Command(spec) if spec.label == "Archive Read Chats"))
        .expect("the sweep command should be listed");
    let Some(crate::palette_items::Entry::Command(spec)) =
        crate::palette_items::entry_at(&snaps, "archive read", gpui_kit::component::IndexPath::new(row).section(0), 0)
    else {
        panic!("entry_at should resolve the sweep row");
    };
    let crate::palette_items::Effect::Run(run) = spec.effect else {
        panic!("the sweep should run, not dispatch");
    };
    cx.update(|window, cx| ws.update(cx, |this, cx| run(this, window, cx)));
    assert!(app.has_pending_prompt(), "the palette row reaches the confirm");
    app.simulate_prompt_answer("Archive");
    app.run_until_parked();
    assert!(archived(&ws, ids[0], &app), "confirming the palette row archives the chat");
}

/// A chat that turns unread while the confirm sits open survives — the
/// predicate is re-checked when the answer lands.
#[test]
fn sweep_rechecks_flags_after_prompt() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let ids = chats(&ws, 2, cx);
    cx.update(|window, cx| ws.update(cx, |this, cx| this.archive_read_chats(window, cx)));
    assert!(app.has_pending_prompt());
    // ids[0] goes unread behind the open prompt.
    app.update(|cx| {
        ws.update(cx, |this, _| {
            let ix = this.chats.iter().position(|c| c.id == ids[0]).unwrap();
            this.chats[ix].unread = true;
        })
    });
    app.simulate_prompt_answer("Archive");
    app.run_until_parked();
    assert!(!archived(&ws, ids[0], &app), "newly-unread chat survives the confirmed sweep");
    assert!(archived(&ws, ids[1], &app), "the still-read chat archives");
}

/// The toast copy names the archived count.
#[test]
fn sweep_toast_counts() {
    assert_eq!(super::sweep_toast(3), "Archived 3 chat(s)");
}
