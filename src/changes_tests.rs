//! Tests for the Changes panel's git actions — real `git` against temp repos
//! (skipped when git is unavailable), plus `parse_branch` fixtures. `gh` is
//! faked with a PATH-override script so `create_pr` is testable without it.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::changes::diff_summary;
    use crate::changes_ui_tests::change;
    use crate::git::{self, BranchStatus, ChangeStatus};

    fn run(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// A temp git repo with one commit and a local identity — the ops need a
    /// HEAD and `commit` needs `user.*`. Returns None when git isn't installed.
    fn temp_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-changes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if !run(&dir, &["init", "-q"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        assert!(run(&dir, &["config", "user.email", "t@t"]));
        assert!(run(&dir, &["config", "user.name", "t"]));
        std::fs::write(dir.join("f.txt"), "one\n").unwrap();
        assert!(run(&dir, &["add", "f.txt"]));
        assert!(run(&dir, &["commit", "-qm", "init"]));
        Some(dir)
    }

    /// A `bin` dir under `parent` exposing only the real `git` — a test that
    /// hides `gh` still needs `git` on the child's PATH, and git usually
    /// shares a directory with it.
    fn git_only_path(parent: &Path) -> String {
        let bin = parent.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let git = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join("git"))
            .find(|p| p.is_file())
            .expect("git on PATH");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&git, bin.join("git")).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&git, bin.join("git")).unwrap();
        format!("{}:/usr/bin:/bin", bin.display())
    }

    /// A `bin` dir under `parent` whose `gh` is a script logging argv to
    /// `log`. Returns the PATH value that puts it first.
    fn fake_gh_path(parent: &Path, log: &Path) -> String {
        let bin = parent.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = format!("#!/bin/sh\necho \"$@\" >> {}\necho https://example.test/pr/7\n", log.display());
        std::fs::write(bin.join("gh"), script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default())
    }

    #[test]
    fn parse_branch_reads_headers() {
        let raw = "# branch.oid abc123\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n";
        assert_eq!(
            git::parse_branch(raw),
            BranchStatus {
                name: "main".into(),
                upstream: Some("origin/main".into()),
                ahead: 2,
                behind: 1
            }
        );
    }

    #[test]
    fn parse_branch_detached_and_unborn() {
        let detached = git::parse_branch("# branch.oid 0e372a90ac2e9bb4f297415a12a092e72cea3f72\n# branch.head (detached)\n");
        assert_eq!(detached.name, "0e372a90");
        assert_eq!(detached.upstream, None);
        let unborn = git::parse_branch("# branch.oid (initial)\n# branch.head main\n");
        assert_eq!(unborn.name, "main");
    }

    #[test]
    fn stage_and_unstage_update_the_index() {
        let Some(dir) = temp_repo("stage") else { return };
        std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
        assert!(!git::collect(&dir)[0].staged, "modified file starts unstaged");

        git::stage(&dir, "f.txt").unwrap();
        let staged = git::collect(&dir);
        assert!(staged[0].staged, "git add staged the file");

        git::unstage(&dir, "f.txt").unwrap();
        assert!(!git::collect(&dir)[0].staged, "restore --staged unstaged it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `restore --staged` can't resolve HEAD on a fresh repo — `unstage` must
    /// fall back to `reset` so a staged new file returns to untracked.
    #[test]
    fn unstage_on_unborn_head_falls_back_to_reset() {
        let dir = std::env::temp_dir().join(format!("rixlcode-changes-unborn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if !run(&dir, &["init", "-q"]) {
            return;
        }
        std::fs::write(dir.join("new.txt"), "hi\n").unwrap();
        git::stage(&dir, "new.txt").unwrap();
        assert!(git::collect(&dir)[0].staged, "add staged the new file");

        git::unstage(&dir, "new.txt").unwrap();
        let after = git::collect(&dir);
        assert_eq!(after.len(), 1);
        assert!(!after[0].staged, "reset returned the file to untracked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_creates_a_commit_with_the_message() {
        let Some(dir) = temp_repo("commit") else { return };
        std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
        git::stage(&dir, "f.txt").unwrap();

        git::commit(&dir, "second commit").unwrap();
        let log = git::git(&dir, &["log", "-1", "--format=%s"]).unwrap_or_default();
        assert_eq!(log.trim(), "second commit");
        assert!(git::collect(&dir).is_empty(), "tree is clean after commit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_publishes_to_a_bare_remote_and_sets_upstream() {
        let Some(dir) = temp_repo("push") else { return };
        let remote = std::env::temp_dir().join(format!("rixlcode-changes-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&remote);
        assert!(run(&dir, &["init", "-q", "--bare", &remote.to_string_lossy()]));
        assert!(run(&dir, &["remote", "add", "origin", &remote.to_string_lossy()]));

        // No upstream yet → push must pass `-u origin HEAD`.
        assert!(git::branch_status(&dir).unwrap().upstream.is_none());
        git::push(&dir).unwrap();

        let branch = git::branch_status(&dir).unwrap();
        assert_eq!(branch.upstream.as_deref(), Some(format!("origin/{}", branch.name).as_str()));
        assert_eq!((branch.ahead, branch.behind), (0, 0));
        let remote_head = git::git(&remote, &["rev-parse", "--verify", &format!("refs/heads/{}", branch.name)]);
        assert!(remote_head.is_some(), "remote has the pushed branch");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&remote);
    }

    #[test]
    fn create_pr_invokes_gh_and_reports_the_url() {
        let Some(dir) = temp_repo("pr") else { return };
        let remote = std::env::temp_dir().join(format!("rixlcode-changes-pr-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&remote);
        assert!(run(&dir, &["init", "-q", "--bare", &remote.to_string_lossy()]));
        assert!(run(&dir, &["remote", "add", "origin", &remote.to_string_lossy()]));

        let log = dir.join("gh-args.log");
        let path = fake_gh_path(&dir, &log);
        let note = git::create_pr(&dir, &[("PATH", path.as_str())]).unwrap();
        assert_eq!(note, "PR created: https://example.test/pr/7");
        let argv = std::fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(argv.trim(), "pr create --fill", "gh got the create args");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&remote);
    }

    /// Without `gh` on PATH the push still lands and the note tells the user
    /// to open the PR by hand.
    #[test]
    fn create_pr_without_gh_pushes_and_notes_the_fallback() {
        let Some(dir) = temp_repo("pr-nogh") else { return };
        let remote = std::env::temp_dir().join(format!("rixlcode-changes-nogh-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&remote);
        assert!(run(&dir, &["init", "-q", "--bare", &remote.to_string_lossy()]));
        assert!(run(&dir, &["remote", "add", "origin", &remote.to_string_lossy()]));

        // PATH with real git but no gh.
        let path = git_only_path(&dir);
        let note = git::create_pr(&dir, &[("PATH", path.as_str())]).unwrap();
        assert_eq!(note, "Pushed — install `gh` to create a PR from here");
        assert!(git::branch_status(&dir).unwrap().upstream.is_some(), "push still ran");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&remote);
    }

    #[test]
    fn branch_status_is_none_outside_a_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-changes-norepo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(git::branch_status(&dir).is_none(), "non-repo dir has no branch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_branches_marks_current_and_drops_detached() {
        let raw = "*\u{0}main\n \u{0}feature\n*\u{0}(HEAD detached at abc1234)\n";
        assert_eq!(
            crate::git_parse::parse_branches(raw),
            vec![
                git::Branch { name: "main".into(), current: true },
                git::Branch { name: "feature".into(), current: false },
            ]
        );
    }

    #[test]
    fn list_branches_returns_locals_with_current_marked() {
        let Some(dir) = temp_repo("list") else { return };
        assert!(run(&dir, &["branch", "feature"]));
        let branches = git::list_branches(&dir);
        assert_eq!(branches.iter().filter(|b| b.current).count(), 1, "exactly one current");
        let current = branches.iter().find(|b| b.current).unwrap();
        assert_eq!(current.name, git::branch_status(&dir).unwrap().name);
        assert!(branches.iter().any(|b| b.name == "feature" && !b.current));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkout_switches_branches() {
        let Some(dir) = temp_repo("checkout") else { return };
        let initial = git::branch_status(&dir).unwrap().name;
        assert!(run(&dir, &["branch", "other"]));
        assert_eq!(git::checkout(&dir, "other").as_deref(), Ok("Switched to other"));
        assert_eq!(git::branch_status(&dir).unwrap().name, "other");
        git::checkout(&dir, &initial).unwrap();
        assert_eq!(git::branch_status(&dir).unwrap().name, initial);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A checkout that would overwrite local edits fails, and the error
    /// names the file so the panel's note tells the user what blocked it.
    #[test]
    fn checkout_refuses_when_edits_would_be_lost() {
        let Some(dir) = temp_repo("dirty") else { return };
        let initial = git::branch_status(&dir).unwrap().name;
        assert!(run(&dir, &["checkout", "-qb", "other"]));
        std::fs::write(dir.join("f.txt"), "other\n").unwrap();
        assert!(run(&dir, &["commit", "-qam", "other"]));
        assert!(run(&dir, &["checkout", "-q", &initial]));
        std::fs::write(dir.join("f.txt"), "dirty\n").unwrap();
        let err = git::checkout(&dir, "other").unwrap_err();
        assert!(err.contains("f.txt"), "error names the blocking file: {err}");
        assert_eq!(git::branch_status(&dir).unwrap().name, initial, "HEAD stayed put");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_branch_makes_and_switches() {
        let Some(dir) = temp_repo("create") else { return };
        assert_eq!(git::create_branch(&dir, "feature-x").as_deref(), Ok("Created feature-x"));
        assert_eq!(git::branch_status(&dir).unwrap().name, "feature-x");
        let branches = git::list_branches(&dir);
        assert!(branches.iter().any(|b| b.name == "feature-x" && b.current));
        assert!(git::create_branch(&dir, "feature-x").is_err(), "duplicate name fails");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Untracked files surface as `Added` rows whose `added` is the on-disk
    /// line count (`git::collect` fills it — `git_tests` covers that), so
    /// they fold into the rollup like any numstat row.
    #[test]
    fn diff_summary_aggregates_numstat_counts() {
        let changes = vec![
            change("src/edited.rs", ChangeStatus::Modified, 3, 1),
            change("src/untracked.rs", ChangeStatus::Added, 12, 0),
            change("src/old.rs", ChangeStatus::Deleted, 0, 8),
        ];
        let summary = diff_summary(&changes).expect("changes present");
        assert_eq!((summary.files, summary.added, summary.deleted), (3, 15, 9));
        assert_eq!(summary.text(), "3 files changed · +15 −9");
    }

    #[test]
    fn diff_summary_is_none_on_a_clean_tree() {
        assert!(diff_summary(&[]).is_none(), "no rows → no header line");
    }
}
