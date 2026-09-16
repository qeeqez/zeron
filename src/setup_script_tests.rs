//! Tests for the per-project setup script — real `git` worktrees and real
//! `sh -c` runs against temp repos (skipped when git is unavailable), plus
//! the headless path that lands a failure as a chat note.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use gpui_kit::component::Root;
    use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

    use crate::model::MessageKind;
    use crate::project::{Project, ProjectState};
    use crate::workspace::Workspace;

    /// Redirect `~` into a throwaway dir so the project store (state.json)
    /// stays off the real profile. nextest runs each test in its own
    /// process, so no other thread can observe HOME mid-write.
    fn sandbox_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
        dir
    }

    /// A temp git repo with one commit — `worktree add` needs a HEAD.
    /// Returns None when git isn't installed.
    fn temp_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-setup-repo-{name}-{}", std::process::id()));
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
        std::fs::write(dir.join("f.txt"), "hi").unwrap();
        assert!(git(&["add", "."]));
        assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
        Some(dir)
    }

    /// Mount a `Workspace` bound to `project` — HOME must already point at
    /// the test's temp dir.
    fn mount(cx: &mut TestAppContext, project: Project) -> (Entity<Workspace>, &mut VisualTestContext) {
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

    /// Poll `cond` with a real-time deadline — the setup script runs on a
    /// spawned thread, so `run_until_parked` alone can't wait for it.
    fn until(cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for the setup script");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn setup_script_runs_in_the_new_worktree() {
        sandbox_home("runs");
        let Some(root) = temp_repo("runs") else { return };
        let project = Project::open(&root);
        project.save_state(&ProjectState {
            setup_script: "touch setup-marker".into(),
            ..Default::default()
        });

        let dir = crate::worktree::create(&project, 1).unwrap();
        let marker = dir.join("setup-marker");
        until(|| marker.exists());
        // Join the run so the spawned thread can't outlive the test
        // process (nextest flags that as leaky).
        if let Some(handle) = crate::setup_script::take_pending(&dir) {
            let _ = handle.join();
        }
        crate::worktree::remove(project.root(), &dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_setup_script_is_skipped() {
        sandbox_home("empty");
        let Some(root) = temp_repo("empty") else { return };
        let project = Project::open(&root);
        // No script configured — spawn is a no-op, nothing lands in PENDING.
        assert!(!crate::setup_script::spawn(&project, &root));
        // Whitespace-only counts as empty too.
        project.save_state(&ProjectState { setup_script: "  \n ".into(), ..Default::default() });
        assert!(!crate::setup_script::spawn(&project, &root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failing_setup_script_lands_a_note_not_a_panic() {
        sandbox_home("fail");
        let Some(root) = temp_repo("fail") else { return };
        let project = Project::open(&root);
        project.save_state(&ProjectState { setup_script: "exit 3".into(), ..Default::default() });

        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app, project.clone());
        cx.update(|_window, cx| {
            ws.update(cx, |this, cx| {
                this.default_workspace = crate::worktree::WorkspaceMode::Worktree;
                this.new_chat(cx);
            });
        });
        let workdir = ws.read_with(cx, |w, _| w.chats[w.active].workdir.clone());
        assert!(workdir.contains(".worktrees"), "the thread should sit in a worktree, got {workdir}");

        // The spawned script finishes in real time; the watcher lands the
        // note on the next executor pass — poll with a real-time deadline.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            let landed = ws.read_with(cx, |w, _| {
                w.chats[w.active]
                    .messages
                    .iter()
                    .any(|m| matches!(&m.kind, MessageKind::Text(t) if t.contains("Setup script failed")))
            });
            if landed {
                break;
            }
            assert!(Instant::now() < deadline, "the failure note never landed");
            std::thread::sleep(Duration::from_millis(10));
        }
        // The failure is also recorded in the activity feed — and the chat
        // kept its worktree.
        let (feed_has_error, still_worktree) = ws.read_with(cx, |w, _| {
            (w.activity.entries.iter().any(|e| e.kind == crate::activity::ActivityKind::Error), w.chats[w.active].worktree)
        });
        assert!(feed_has_error, "the feed should record the failed setup");
        assert!(still_worktree);
        crate::worktree::remove(project.root(), std::path::Path::new(&workdir));
        let _ = std::fs::remove_dir_all(&root);
    }
}
