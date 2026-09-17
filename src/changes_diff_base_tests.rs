//! Tests for the worktree diff-base picker — ref listing, base resolution
//! (default, picked, stale), the scratch-index change list, persistence of
//! `Chat::diff_base`, and the panel's pick→recompute path. Real temp repos,
//! skipped when git is unavailable.

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

    use crate::project::Project;
    use crate::workspace::Workspace;
    use crate::worktree::diff::{resolve_diff_base, worktree_changes};

    /// A temp git repo with one commit — `worktree add` needs a HEAD.
    /// Returns None when git isn't installed.
    fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-dbase-{name}-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("rixlcode-dbase-home-{name}-{}", std::process::id()));
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

    /// Wait until `cond` holds — collection runs real git on the background
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
    fn list_diff_bases_covers_branches_and_tags() {
        let Some(dir) = temp_repo("refs") else { return };
        git(&dir, &["branch", "feature"]);
        git(&dir, &["tag", "v1.0"]);
        let refs = crate::git::list_diff_bases(&dir);
        let head = crate::git::git(&dir, &["symbolic-ref", "--short", "HEAD"]).unwrap();
        let head = head.trim();
        assert!(refs.iter().any(|r| r.name == head && !r.tag), "current branch listed: {refs:?}");
        assert!(refs.iter().any(|r| r.name == "feature" && !r.tag), "branch listed: {refs:?}");
        assert!(refs.iter().any(|r| r.name == "v1.0" && r.tag), "tag listed: {refs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_name_status_rows() {
        let rows = crate::worktree::diff::parse_name_status("M\0f.txt\0A\0new.txt\0R100\0old.txt\0renamed.txt\0D\0gone.txt\0");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].status, crate::git::ChangeStatus::Modified);
        assert_eq!(rows[1].status, crate::git::ChangeStatus::Added);
        assert!(!rows[1].staged, "base-diff rows carry no staged marker");
        assert_eq!(rows[2].status, crate::git::ChangeStatus::Renamed);
        assert_eq!(rows[2].source.as_deref(), Some("old.txt"), "rename keeps its source path");
        assert_eq!(rows[3].status, crate::git::ChangeStatus::Deleted);
    }

    #[test]
    fn resolve_diff_base_defaults_to_project_head() {
        let Some(root) = temp_repo("default") else { return };
        let project = Project::open(&root);
        let wt = crate::worktree::create(&project, 1).unwrap();
        let base = resolve_diff_base(project.root(), &wt, None).unwrap();
        let head = crate::git::git(&root, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(base.commit, head.trim(), "fresh worktree's merge-base is HEAD");
        assert!(!base.stale);
        let branch = crate::git::git(&root, &["symbolic-ref", "--short", "HEAD"]).unwrap();
        assert_eq!(base.label, branch.trim(), "the label names the project's branch");
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_base_falls_back_to_default() {
        let Some(root) = temp_repo("stale") else { return };
        let project = Project::open(&root);
        let wt = crate::worktree::create(&project, 2).unwrap();
        let base = resolve_diff_base(project.root(), &wt, Some("deleted-branch")).unwrap();
        let head = crate::git::git(&root, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(base.commit, head.trim(), "stale pick falls back to the default base");
        assert!(base.stale, "the panel notes the fallback");
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn worktree_changes_diffs_against_the_picked_base() {
        let Some(root) = temp_repo("pick") else { return };
        let project = Project::open(&root);
        // A branch one commit behind HEAD: the worktree's HEAD contains the
        // second commit, so merge-base(wt, old) is the first commit.
        git(&root, &["branch", "old"]);
        std::fs::write(root.join("f.txt"), "line1\nline2\nline3\nroot\n").unwrap();
        git(&root, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qam", "second"]);
        let wt = crate::worktree::create(&project, 3).unwrap();
        std::fs::write(wt.join("wt.txt"), "worktree\n").unwrap();

        // Default base (merge-base with project HEAD) shows only the
        // worktree's own file.
        let base = resolve_diff_base(project.root(), &wt, None).unwrap();
        let changes = worktree_changes(&wt, &base.commit).unwrap();
        assert_eq!(changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["wt.txt"]);
        assert_eq!(changes[0].status, crate::git::ChangeStatus::Added);
        assert_eq!(changes[0].added, 1, "untracked content counts via the scratch index");

        // Picking `old` moves the base back a commit — the root's second
        // commit lands in the diff too.
        let base = resolve_diff_base(project.root(), &wt, Some("old")).unwrap();
        let changes = worktree_changes(&wt, &base.commit).unwrap();
        let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["f.txt", "wt.txt"], "the picked base widens the diff: {paths:?}");
        assert_eq!(changes[0].status, crate::git::ChangeStatus::Modified);
        assert_eq!((changes[0].added, changes[0].deleted), (1, 0));
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn diff_base_persists() {
        let dir = std::env::temp_dir().join(format!("rixlcode-dbase-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut chat = crate::model::Chat::new(0, "wt");
        chat.worktree = true;
        chat.diff_base = Some("release/1.2".into());
        crate::persist::save_chats(&dir, &[chat]);
        let mut next = 1;
        let loaded = crate::persist::load_chats(&dir, &mut next, false);
        assert_eq!(loaded[0].diff_base.as_deref(), Some("release/1.2"));
        // Files written before the picker existed carry no field.
        std::fs::write(dir.join("0.json"), r#"{"v":1,"title":"old","messages":[]}"#).unwrap();
        let mut next = 1;
        let loaded = crate::persist::load_chats(&dir, &mut next, false);
        assert_eq!(loaded[0].diff_base, None, "missing field loads as the default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Picking a base on a worktree chat recomputes the panel's file list
    /// against that ref's merge-base — the UI path end to end.
    #[test]
    fn picking_a_base_recomputes_the_panel() {
        let Some(root) = temp_repo("panel") else { return };
        let project = Project::open(&root);
        git(&root, &["branch", "old"]);
        std::fs::write(root.join("f.txt"), "line1\nline2\nline3\nroot\n").unwrap();
        git(&root, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qam", "second"]);
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app, "panel", project.clone());
        let wt = crate::worktree::create(&project, 4).unwrap();
        std::fs::write(wt.join("wt.txt"), "worktree\n").unwrap();
        cx.update(|_, cx| {
            ws.update(cx, |this, cx| {
                let chat = &mut this.chats[this.active];
                chat.worktree = true;
                chat.workdir = wt.to_string_lossy().into_owned();
                this.refresh_changes(cx);
            });
        });
        until(&ws, cx, |ws| ws.changes_scope().base.is_some());
        ws.read_with(cx, |ws, _| {
            assert_eq!(ws.changes.len(), 1, "default base shows only the worktree file");
            assert_eq!(ws.changes[0].path, "wt.txt");
        });
        cx.update(|_, cx| {
            ws.update(cx, |this, cx| this.set_diff_base(Some("old".into()), cx));
        });
        until(&ws, cx, |ws| ws.changes.len() == 2);
        ws.read_with(cx, |ws, _| {
            let paths: Vec<&str> = ws.changes.iter().map(|c| c.path.as_str()).collect();
            assert_eq!(paths, ["f.txt", "wt.txt"], "the picked base widened the diff");
            assert_eq!(ws.changes_scope().base.as_ref().unwrap().label, "old");
            assert_eq!(ws.chats[ws.active].diff_base.as_deref(), Some("old"), "the pick persisted on the chat");
        });
        crate::worktree::remove(project.root(), &wt);
        let _ = std::fs::remove_dir_all(&root);
    }
}
