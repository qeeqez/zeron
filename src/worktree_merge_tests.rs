//! Tests for worktree merge-back — `merge_into` against real temp repos
//! (skipped when git is unavailable), plus the ⋯ menu's disabled-while-
//! running gate and the merge → remove-worktree follow-through.

#[cfg(test)]
mod tests {
    use gpui_kit::base::test_support::snapshots;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

    use crate::project::Project;
    use crate::workspace::Workspace;
    use crate::worktree::merge::{MergeOutcome, merge_into};

    /// A temp git repo with one commit — `worktree add` needs a HEAD.
    /// Returns None when git isn't installed.
    fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-wtm-{name}-{}", std::process::id()));
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
        std::fs::write(dir.join("f.txt"), "line1\nline2\nline3\n").unwrap();
        assert!(git(&["add", "."]));
        assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
        Some(dir)
    }

    /// `git` in `dir`, asserting success — test-side setup only.
    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed");
    }

    /// Mount a `Workspace` bound to `project` — HOME must already point at
    /// the test's temp dir.
    fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
        let dir = std::env::temp_dir().join(format!("rixlcode-wtm-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: nextest runs each test in its own process, so no other
        // thread can observe HOME mid-write.
        unsafe { std::env::set_var("HOME", &dir) };
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

    /// The ⋯ menu on the chat titlebar — opened by clicking the header button.
    fn open_chat_menu(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.click("chat-menu", cx);
            window.draw(cx).clear(cx);
            assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
        });
    }

    /// Click the popup-menu item with `label` — panics when it isn't offered.
    fn click_menu_item(window: &mut Window, label: &str, cx: &mut App) {
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some(label))
            .unwrap_or_else(|| panic!("menu should offer {label}"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").click(id, cx);
    }

    /// Wait until `cond` holds — the merge runs real git on the background
    /// executor, so pump and sleep like `until_issued` does.
    fn until(ws: &Entity<Workspace>, cx: &mut VisualTestContext, cond: impl Fn(&Workspace) -> bool) {
        for _ in 0..200 {
            cx.run_until_parked();
            if ws.read_with(cx, |ws, _| cond(ws)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("condition never held");
    }

    #[test]
    fn merge_into_applies_uncommitted_and_untracked_work() {
        let Some(root) = temp_repo("apply") else { return };
        let project = Project::open(&root);
        let wt = crate::worktree::create(&project, 1).unwrap();
        std::fs::write(wt.join("f.txt"), "line1\nwt-change\nline3\n").unwrap();
        std::fs::write(wt.join("added.txt"), "new\n").unwrap();

        assert_eq!(merge_into(project.root(), &wt), MergeOutcome::Applied);
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "line1\nwt-change\nline3\n");
        assert_eq!(std::fs::read_to_string(root.join("added.txt")).unwrap(), "new\n");
        // The worktree is untouched — its files still hold the work.
        assert!(wt.join("added.txt").exists());
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_into_applies_committed_work() {
        let Some(root) = temp_repo("commit") else { return };
        let project = Project::open(&root);
        let wt = crate::worktree::create(&project, 2).unwrap();
        std::fs::write(wt.join("f.txt"), "line1\ncommitted\nline3\n").unwrap();
        git(&wt, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qam", "wip"]);

        assert_eq!(merge_into(project.root(), &wt), MergeOutcome::Applied);
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "line1\ncommitted\nline3\n");
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_into_conflict_keeps_worktree_and_reports_paths() {
        let Some(root) = temp_repo("conflict") else { return };
        let project = Project::open(&root);
        let wt = crate::worktree::create(&project, 3).unwrap();
        std::fs::write(wt.join("f.txt"), "line1\nwt-change\nline3\n").unwrap();
        // The root commits a conflicting edit to the same line.
        std::fs::write(root.join("f.txt"), "line1\nroot-change\nline3\n").unwrap();
        git(&root, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qam", "root-edit"]);

        match merge_into(project.root(), &wt) {
            MergeOutcome::Conflicted(paths) => assert_eq!(paths, ["f.txt"]),
            other => panic!("expected conflicts, got {other:?}"),
        }
        // The worktree survives with its work intact.
        assert!(wt.exists());
        assert_eq!(std::fs::read_to_string(wt.join("f.txt")).unwrap(), "line1\nwt-change\nline3\n");
        // The root carries the conflict markers + unmerged index entry.
        let merged = std::fs::read_to_string(root.join("f.txt")).unwrap();
        assert!(merged.contains("<<<<<<<"), "conflict markers landed: {merged}");
        let unmerged = crate::git::git(&root, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        assert!(unmerged.contains("f.txt"), "unmerged: {unmerged}");
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_into_empty_when_no_delta() {
        let Some(root) = temp_repo("empty") else { return };
        let project = Project::open(&root);
        let wt = crate::worktree::create(&project, 4).unwrap();
        assert_eq!(merge_into(project.root(), &wt), MergeOutcome::Empty);
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_item_disabled_while_turn_runs() {
        let Some(root) = temp_repo("gate") else { return };
        let project = Project::open(&root);
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app, "gate", project.clone());
        // Mount first, then create — `for_project` prunes clean orphans on
        // open, so the worktree must exist after its chat claims it.
        let wt = crate::worktree::create(&project, 5).unwrap();
        std::fs::write(wt.join("f.txt"), "line1\nwt-change\nline3\n").unwrap();
        cx.update(|_, cx| {
            ws.update(cx, |this, _| {
                let chat = &mut this.chats[this.active];
                chat.worktree = true;
                chat.workdir = wt.to_string_lossy().into_owned();
                chat.running = true;
            });
        });
        // The item is offered but disabled — clicking it merges nothing.
        open_chat_menu(cx);
        cx.update(|window, cx| click_menu_item(window, "Merge into project", cx));
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "line1\nline2\nline3\n", "a running turn blocks the merge");
        ws.read_with(cx, |this, _| assert!(!this.git.busy, "no merge op started"));
        // A disabled item doesn't dismiss the menu — close it by hand.
        cx.update(|window, cx| window.press("escape", cx));
        // Once the turn ends the same click lands the work.
        cx.update(|_, cx| {
            ws.update(cx, |this, _| this.chats[this.active].running = false);
        });
        open_chat_menu(cx);
        cx.update(|window, cx| click_menu_item(window, "Merge into project", cx));
        until(&ws, cx, |ws| !ws.git.busy);
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "line1\nwt-change\nline3\n");
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_menu_item_applies_then_removes_clean_worktree() {
        let Some(root) = temp_repo("e2e") else { return };
        let project = Project::open(&root);
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app, "e2e", project.clone());
        let wt = crate::worktree::create(&project, 6).unwrap();
        // Committed work — the worktree is clean, so the merge offers
        // removal afterwards.
        std::fs::write(wt.join("merged.txt"), "from worktree\n").unwrap();
        git(&wt, &["add", "merged.txt"]);
        git(&wt, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "wip"]);
        cx.update(|_, cx| {
            ws.update(cx, |this, _| {
                let chat = &mut this.chats[this.active];
                chat.worktree = true;
                chat.workdir = wt.to_string_lossy().into_owned();
            });
        });
        open_chat_menu(cx);
        cx.update(|window, cx| click_menu_item(window, "Merge into project", cx));
        until(&ws, cx, |ws| !ws.git.busy);
        assert_eq!(std::fs::read_to_string(root.join("merged.txt")).unwrap(), "from worktree\n");
        // Clean worktree → the removal prompt; confirming drops the dir and
        // the chat's worktree flags.
        assert!(app.has_pending_prompt(), "clean merge offers worktree removal");
        app.simulate_prompt_answer("Remove");
        app.run_until_parked();
        assert!(!wt.exists(), "confirmed removal drops the worktree");
        app.read(|cx| {
            ws.read_with(cx, |this, _| {
                let chat = &this.chats[this.active];
                assert!(!chat.worktree, "removed worktree clears the flag");
                assert_eq!(chat.workdir, project.root().to_string_lossy(), "chat runs in the root again");
            });
        });
        let _ = std::fs::remove_dir_all(&root);
    }
}
