//! Tests for the Snapshots surface: metadata collection over real git temp
//! repos (skipped when git is unavailable, same as `checkpoints_tests`),
//! restore/delete against the workdir, the retention policy, and the
//! headless panel's list + row actions.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use crate::checkpoints::{Checkpoint, TurnCheckpoint, restore, snapshot};
    use crate::snapshot_store::{SnapshotStatus, delete, describe_files};
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

    /// The expanded row's path list for a git checkpoint: modified, added
    /// (untracked — restore deletes it) and deleted paths, sorted.
    #[test]
    fn describe_files_lists_git_paths() {
        let Some(repo) = temp_repo("paths") else { return };
        std::fs::write(repo.join("gone.txt"), "gone").unwrap();
        let checkpoint = snapshot(&repo, &temp_dir("store"), "c1-0").unwrap();
        std::fs::write(repo.join("base.txt"), "edited").unwrap();
        std::fs::write(repo.join("new.txt"), "new").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        let (_, files) = describe_files(&repo, &checkpoint);
        let files = files.expect("diff computable");
        let got: Vec<(&str, SnapshotStatus)> = files.iter().map(|f| (f.path.as_str(), f.status)).collect();
        assert_eq!(
            got,
            [
                ("base.txt", SnapshotStatus::Modified),
                ("gone.txt", SnapshotStatus::Deleted),
                ("new.txt", SnapshotStatus::Added),
            ]
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// Same list for a copy checkpoint — the dir-compare walk, including a
    /// nested path and an added directory (listed as `dir/`).
    #[test]
    fn describe_files_lists_copy_paths() {
        let workdir = temp_dir("copy-src");
        std::fs::create_dir_all(workdir.join("sub")).unwrap();
        std::fs::write(workdir.join("sub/keep.txt"), "keep").unwrap();
        std::fs::write(workdir.join("gone.txt"), "gone").unwrap();
        let checkpoint = snapshot(&workdir, &temp_dir("copy-store"), "c1-0").unwrap();
        std::fs::write(workdir.join("sub/keep.txt"), "edited").unwrap();
        std::fs::remove_file(workdir.join("gone.txt")).unwrap();
        std::fs::create_dir_all(workdir.join("added")).unwrap();
        std::fs::write(workdir.join("added/new.txt"), "new").unwrap();
        let (_, files) = describe_files(&workdir, &checkpoint);
        let files = files.expect("diff computable");
        let got: Vec<(&str, SnapshotStatus)> = files.iter().map(|f| (f.path.as_str(), f.status)).collect();
        assert_eq!(
            got,
            [
                ("added/", SnapshotStatus::Added),
                ("added/new.txt", SnapshotStatus::Added),
                ("gone.txt", SnapshotStatus::Deleted),
                ("sub/keep.txt", SnapshotStatus::Modified),
            ]
        );
        // A deleted copy dir reports `None` — the row shows "—".
        let Checkpoint::Copy(dir) = &checkpoint else { panic!("copy checkpoint") };
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(describe_files(&workdir, &checkpoint).1, None);
        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// A snapshot matching the workdir lists no files — the row's restore
    /// stays one-click.
    #[test]
    fn describe_files_empty_when_clean() {
        let Some(repo) = temp_repo("clean") else { return };
        let checkpoint = snapshot(&repo, &temp_dir("store"), "c1-0").unwrap();
        let (_, files) = describe_files(&repo, &checkpoint);
        assert_eq!(files, Some(vec![]));
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

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "snapshots_ui_tests.rs"]
mod ui;
