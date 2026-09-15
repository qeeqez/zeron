//! Tests for the Snapshots surface: metadata collection over real git temp
//! repos (skipped when git is unavailable, same as `checkpoints_tests`),
//! restore/delete against the workdir, the retention policy, and the
//! headless panel's list + row actions.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use crate::checkpoints::{Checkpoint, TurnCheckpoint, restore, snapshot};
    use crate::snapshot_store::delete;
    use crate::snapshots::{ChatSeed, collect, prune};

    /// A temp git repo with one committed file — `None` when git isn't
    /// installed. `name` keeps parallel test dirs apart.
    fn temp_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-snap-{}-{name}", std::process::id()));
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-snap-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// One chat's worth of seeds around a single checkpoint.
    fn seed(id: u64, title: &str, workdir: &Path, turn: TurnCheckpoint) -> ChatSeed {
        ChatSeed {
            id,
            title: title.to_string(),
            workdir: workdir.to_path_buf(),
            checkpoints: vec![turn],
        }
    }

    #[test]
    fn collect_lists_metadata_newest_first() {
        let Some(repo) = temp_repo("list") else { return };
        let older = SystemTime::now() - Duration::from_secs(3600);
        let first = snapshot(&repo, &temp_dir("store"), "c1-0").unwrap();
        std::fs::write(repo.join("new.txt"), "new").unwrap();
        let second = snapshot(&repo, &temp_dir("store"), "c1-1").unwrap();
        let seeds = vec![
            seed(1, "First chat", &repo, TurnCheckpoint { ix: 0, at: older, checkpoint: first }),
            seed(2, "Second chat", &repo, TurnCheckpoint { ix: 1, at: SystemTime::now(), checkpoint: second }),
        ];
        let list = collect(&seeds);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].chat_title, "Second chat", "newest first");
        assert_eq!(list[0].chat_id, 2);
        assert_eq!(list[0].message_ix, 1);
        assert!(list[0].bytes > 0, "snapshot has content size");
        // The second snapshot contains new.txt; the workdir still matches it.
        assert_eq!(list[0].changed, Some(0));
        // The first predates new.txt — restoring it would remove that file.
        assert_eq!(list[1].changed, Some(1));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn delete_frees_git_snapshot() {
        let Some(repo) = temp_repo("del") else { return };
        let Some(Checkpoint::Git(sha)) = snapshot(&repo, &temp_dir("store"), "c1-0") else { panic!("git snapshot") };
        let cat = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        assert!(cat(&["cat-file", "-e", &sha]), "commit kept alive by the ref");
        delete(&repo, &Checkpoint::Git(sha.clone())).unwrap();
        assert!(!cat(&["cat-file", "-e", &sha]), "gc freed the snapshot commit");
        assert!(restore(&repo, &Checkpoint::Git(sha)).is_err(), "deleted snapshot can't restore");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn delete_frees_copy_snapshot() {
        let workdir = temp_dir("copy-src");
        std::fs::write(workdir.join("a.txt"), "a").unwrap();
        let store = temp_dir("copy-store");
        let Some(Checkpoint::Copy(dir)) = snapshot(&workdir, &store, "c1-0") else { panic!("copy snapshot") };
        assert!(dir.join("a.txt").exists());
        delete(&workdir, &Checkpoint::Copy(dir.clone())).unwrap();
        assert!(!dir.exists(), "copy dir removed");
        let _ = std::fs::remove_dir_all(&workdir);
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn prune_respects_age_and_cap() {
        let repo = temp_dir("prune");
        let now = SystemTime::now();
        let mk = |id: u64, age_days: u64, bytes: u64| {
            let at = now - Duration::from_secs(age_days * 86_400);
            let turn = TurnCheckpoint {
                ix: 0,
                at,
                checkpoint: Checkpoint::Copy(repo.join(format!("s{id}"))),
            };
            let seeds = vec![seed(id, "chat", &repo, turn)];
            let mut info = collect(&seeds).remove(0);
            info.bytes = bytes;
            info
        };
        // Newest first: fresh, week-old, month-old.
        let list = vec![mk(1, 0, 100), mk(2, 7, 100), mk(3, 40, 100)];
        // Age rule: 30 days drops only the month-old snapshot.
        let victims = prune(&list, Some(Duration::from_secs(30 * 86_400)), None);
        assert_eq!(victims.len(), 1);
        assert_eq!(victims[0].chat_id, 3);
        // Cap rule: 150 bytes keeps the newest, drops the rest.
        let victims = prune(&list, None, Some(150));
        assert_eq!(victims.len(), 2);
        assert_eq!(victims[0].chat_id, 2);
        assert_eq!(victims[1].chat_id, 3);
        // Both disabled: nothing pruned.
        assert!(prune(&list, None, None).is_empty());
        let _ = std::fs::remove_dir_all(&repo);
    }
}

#[cfg(test)]
mod ui {
    use std::path::PathBuf;

    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

    use crate::checkpoints::{TurnCheckpoint, snapshot};
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
        // The "turn" edits the workdir; Restore reverts it.
        std::fs::write(repo.join("agent.txt"), "agent").unwrap();
        std::fs::write(repo.join("base.txt"), "agent").unwrap();
        cx.update(|window, cx| {
            window.click(("snapshot-restore", 0usize), cx);
        });
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
}
