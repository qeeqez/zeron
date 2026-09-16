//! Tests for the settings "Thread worktrees" list: `list_live` matches
//! dirs under `.worktrees/` to their owning chats and flags orphans, and
//! the Project section's rows reveal in Finder / delete orphans behind a
//! confirm. The `open_in` recorder asserts reveal argv; `worktree`'s
//! `GIT_ARGV` seam asserts the `git worktree remove` argv — no apps spawn.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::open_in::{ISSUED, reveal_command};
use crate::project::Project;
use crate::workspace::Workspace;
use crate::worktree::{GIT_ARGV, list_live};

/// A fresh temp dir (HOME and project roots both live under it).
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-wtlist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mount a `Workspace` bound to `project` — HOME must already point at the
/// test's temp dir.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", temp_dir(name)) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::for_project(project, window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// Open settings and switch to the Project section.
fn open_project_section(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.open_settings(window, cx));
    window.draw(cx).clear(cx);
    window.click("settings-nav-project", cx);
    window.draw(cx).clear(cx);
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

/// Mount a workspace whose project has one live worktree dir owned by the
/// active chat plus one orphan dir; returns both paths.
fn mount_with_worktrees<'a>(
    app: &'a mut TestAppContext, name: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext, std::path::PathBuf, std::path::PathBuf) {
    let root = temp_dir(&format!("{name}-proj"));
    let project = Project::open(&root);
    let live = project.worktrees_dir().join("thread-live");
    let orphan = project.worktrees_dir().join("thread-orphan");
    std::fs::create_dir_all(&live).unwrap();
    std::fs::create_dir_all(&orphan).unwrap();
    let (ws, cx) = mount(app, name, project);
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let chat = &mut this.chats[this.active];
            chat.title = "Owned thread".into();
            chat.worktree = true;
            chat.workdir = live.to_string_lossy().into_owned();
        });
    });
    (ws, cx, live, orphan)
}

#[test]
fn list_live_reports_dirs_and_orphans() {
    let root = temp_dir("list");
    let live = root.join(".worktrees/thread-1");
    let orphan = root.join(".worktrees/thread-2");
    std::fs::create_dir_all(&live).unwrap();
    std::fs::create_dir_all(&orphan).unwrap();
    // A stray file inside .worktrees is not a checkout.
    std::fs::write(root.join(".worktrees/scratch.txt"), "x").unwrap();
    let mut chat = crate::model::Chat::new(1, "Fix the bug");
    chat.workdir = live.to_string_lossy().into_owned();
    chat.worktree = true;

    let found = list_live(&root, &[chat]);
    assert_eq!(found.len(), 2, "dirs only, sorted by path: {found:?}");
    assert_eq!(found[0].path, live);
    assert_eq!(found[0].chat_title.as_deref(), Some("Fix the bug"));
    assert_eq!(found[1].path, orphan);
    assert_eq!(found[1].chat_title, None, "no chat owns the dir → orphan");

    // A missing .worktrees dir is an empty list, not an error.
    let _ = std::fs::remove_dir_all(root.join(".worktrees"));
    assert!(list_live(&root, &[]).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn worktree_rows_list_live_and_orphan_dirs() {
    let mut app = TestAppContext::single();
    let (ws, cx, _live, _orphan) = mount_with_worktrees(&mut app, "rows");
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        assert!(window.find("worktree-row-thread-live").visible(), "live worktree row renders");
        assert!(window.find("worktree-row-thread-orphan").visible(), "orphan row renders");
        assert!(window.find("worktree-reveal-thread-live").visible(), "every row reveals");
        assert!(window.find("worktree-reveal-thread-orphan").visible());
        // Removing a live thread's checkout would break it — orphans only.
        assert!(window.try_find("worktree-remove-thread-live").is_none(), "live worktree offers no remove");
        assert!(window.find("worktree-remove-thread-orphan").visible(), "orphan offers remove");
    });
}

#[test]
fn empty_state_shows_no_worktrees() {
    let mut app = TestAppContext::single();
    let project = Project::open(temp_dir("empty-proj"));
    let (ws, cx) = mount(&mut app, "empty", project);
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        assert!(window.find("worktree-empty").visible(), "empty state renders");
        assert!(window.try_find("worktree-row-thread-live").is_none());
    });
}

#[test]
fn reveal_issues_open_dash_r() {
    let mut app = TestAppContext::single();
    let (ws, cx, _live, orphan) = mount_with_worktrees(&mut app, "reveal");
    ISSUED.lock().clear();
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        window.click("worktree-reveal-thread-orphan", cx);
    });
    until_issued(cx, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(&orphan)]);
}

#[test]
fn orphan_remove_confirms_then_runs_git_worktree_remove() {
    let mut app = TestAppContext::single();
    let (ws, cx, _live, orphan) = mount_with_worktrees(&mut app, "delete");
    GIT_ARGV.lock().clear();
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        window.click("worktree-remove-thread-orphan", cx);
    });
    // `cx`'s borrow of `app` ends here — the prompt helpers live on `app`.
    assert!(app.has_pending_prompt(), "remove asks for confirmation");
    app.simulate_prompt_answer("Remove");
    app.run_until_parked();
    assert!(!orphan.exists(), "the orphan dir is gone");
    let argv = GIT_ARGV.lock();
    let remove = argv.iter().find(|a| a.starts_with(&["worktree".to_string(), "remove".to_string()]));
    let remove = remove.unwrap_or_else(|| panic!("expected `git worktree remove`, got {argv:?}"));
    let argv: Vec<&str> = remove.iter().map(String::as_str).collect();
    assert_eq!(argv, ["worktree", "remove", &*orphan.to_string_lossy()], "orphan remove runs git worktree remove");
}

#[test]
fn orphan_remove_cancel_leaves_the_dir() {
    let mut app = TestAppContext::single();
    let (ws, cx, _live, orphan) = mount_with_worktrees(&mut app, "cancel");
    GIT_ARGV.lock().clear();
    cx.update(|window, cx| {
        open_project_section(&ws, window, cx);
        window.click("worktree-remove-thread-orphan", cx);
    });
    assert!(app.has_pending_prompt(), "remove asks for confirmation");
    app.simulate_prompt_answer("Cancel");
    app.run_until_parked();
    assert!(orphan.exists(), "cancelled remove keeps the dir");
    // Rendering the list probes each dir (rev-parse/status) — the check is
    // that no REMOVAL ran, not that git never ran.
    assert!(
        !GIT_ARGV.lock().iter().any(|a| a.starts_with(&["worktree".to_string(), "remove".to_string()])),
        "no worktree remove on cancel"
    );
}
