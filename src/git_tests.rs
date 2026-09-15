//! Tests for `git` parsing — sibling file so `git.rs` stays under the SLOC
//! cap. All fixtures are canned command output; no real git runs in tests.

#[cfg(test)]
mod tests {
    use crate::git::ChangeStatus;
    use crate::git_parse::{parse_commit_diff, parse_log, parse_numstat, parse_status};

    #[test]
    fn status_parses_each_change_kind() {
        let raw = " M src/a.rs\0A  src/b.rs\0D  src/c.rs\0?? src/new.rs\0";
        let got = parse_status(raw);
        let kinds: Vec<(&str, ChangeStatus)> = got.iter().map(|c| (c.path.as_str(), c.status)).collect();
        assert_eq!(
            kinds,
            [
                ("src/a.rs", ChangeStatus::Modified),
                ("src/b.rs", ChangeStatus::Added),
                ("src/c.rs", ChangeStatus::Deleted),
                ("src/new.rs", ChangeStatus::Added),
            ]
        );
    }

    #[test]
    fn status_rename_consumes_source_field() {
        // -z rename entries are `R  <new>\0<old>\0` — the old path must not
        // surface as its own row, and it is kept for the rename diff.
        let raw = "R  sub/dst.rs\0sub/src.rs\0 M other.rs\0";
        let got = parse_status(raw);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].path, "sub/dst.rs");
        assert_eq!(got[0].source.as_deref(), Some("sub/src.rs"));
        assert_eq!(got[0].status, ChangeStatus::Renamed);
        assert_eq!(got[1].path, "other.rs");
    }

    #[test]
    fn status_deleted_rename_still_consumes_source_field() {
        // `RD` (staged rename, deleted in worktree) displays as Deleted, but
        // porcelain still emits the source field — leaving it unconsumed
        // parses the old path as a bogus second row.
        let raw = "RD sub/dst.rs\0sub/src.rs\0 M other.rs\0";
        let got = parse_status(raw);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].path, "sub/dst.rs");
        assert_eq!(got[0].source.as_deref(), Some("sub/src.rs"));
        assert_eq!(got[0].status, ChangeStatus::Deleted);
        assert_eq!(got[1].path, "other.rs");
    }

    #[test]
    fn status_conflict_and_combined_codes() {
        let raw = "UU both.rs\0AM added_mod.rs\0MM staged_mod.rs\0";
        let got = parse_status(raw);
        assert_eq!(got[0].status, ChangeStatus::Conflicted);
        assert_eq!(got[1].status, ChangeStatus::Added);
        assert_eq!(got[2].status, ChangeStatus::Modified);
    }

    #[test]
    fn status_ignores_blank_and_short_fields() {
        assert!(parse_status("").is_empty());
        assert!(parse_status("\0\0").is_empty());
        let got = parse_status(" M ok.rs\0x\0");
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn numstat_parses_counts_and_binary() {
        let raw = "10\t2\tsrc/a.rs\0-\t-\timg.png\0";
        let got = parse_numstat(raw);
        assert_eq!(got["src/a.rs"], (10, 2));
        assert_eq!(got["img.png"], (0, 0));
    }

    #[test]
    fn numstat_rename_uses_new_path() {
        // -z rename rows are `a\td\t\0<old>\0<new>\0`.
        let raw = "3\t1\t\0old/name.rs\0new/name.rs\0";
        let got = parse_numstat(raw);
        assert_eq!(got.len(), 1);
        assert_eq!(got["new/name.rs"], (3, 1));
    }

    #[test]
    fn numstat_path_with_tab_survives() {
        let raw = "4\t5\tweird\tname.rs\0";
        let got = parse_numstat(raw);
        assert_eq!(got["weird\tname.rs"], (4, 5));
    }

    #[test]
    fn numstat_empty_is_empty() {
        assert!(parse_numstat("").is_empty());
    }

    /// End-to-end against a real repo: untracked files get a disk line count,
    /// and a non-repo directory yields an empty list rather than a panic.
    /// Skips when `git` is unavailable.
    #[test]
    fn collect_lists_untracked_and_handles_non_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(crate::git::collect(&dir).is_empty(), "non-repo dir yields no changes");

        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            std::fs::write(dir.join("untracked.txt"), "one\ntwo\nthree\n").unwrap();
            let got = crate::git::collect(&dir);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].path, "untracked.txt");
            assert_eq!(got[0].status, ChangeStatus::Added);
            assert_eq!(got[0].added, 3);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A partially-staged file (`MM`) must report the net HEAD→worktree
    /// delta — staged + unstaged combined — not just the staged half.
    /// Skips when `git` is unavailable.
    #[test]
    fn collect_reports_net_counts_for_partially_staged() {
        let dir = std::env::temp_dir().join(format!("rixlcode-git-mm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if run(&["init", "-q"]) {
            std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\n").unwrap();
            run(&["add", "f.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
            // Stage one change, then make another unstaged — status `MM`.
            std::fs::write(dir.join("f.txt"), "ONE\ntwo\nthree\n").unwrap();
            run(&["add", "f.txt"]);
            std::fs::write(dir.join("f.txt"), "ONE\ntwo\nthree\nfour\n").unwrap();

            let got = crate::git::collect(&dir);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].status, ChangeStatus::Modified);
            assert_eq!((got[0].added, got[0].deleted), (2, 1), "net HEAD→worktree delta");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_parses_nul_separated_fields() {
        let got = parse_log("abc1234\x00first commit\x00Alice\x002 hours ago\ndef5678\x00fix: thing\x00Bob\x003 days ago\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].hash, "abc1234");
        assert_eq!(got[0].subject, "first commit");
        assert_eq!(got[0].author, "Alice");
        assert_eq!(got[0].rel_time, "2 hours ago");
        assert_eq!(got[1].hash, "def5678");
    }

    #[test]
    fn log_skips_malformed_and_empty_lines() {
        let got = parse_log("\nabc1234\0subj\n\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].hash, "abc1234");
        assert_eq!(got[0].subject, "subj");
        assert_eq!(got[0].author, "", "missing fields default empty");
    }

    #[test]
    fn commit_diff_splits_files_and_drops_binary() {
        let raw = "diff --git a/f.txt b/f.txt\nindex 1..2 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/g.bin b/g.bin\nindex 3..4 100644\nBinary files a/g.bin and b/g.bin differ\n";
        let got = parse_commit_diff(raw);
        assert_eq!(got.files.len(), 1, "binary-only section dropped");
        assert_eq!(got.files[0].path, "f.txt");
        let kinds: Vec<_> = got.files[0].diff.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                crate::changes_diff::DiffLineKind::Hunk,
                crate::changes_diff::DiffLineKind::Removed,
                crate::changes_diff::DiffLineKind::Added
            ]
        );
    }

    #[test]
    fn commit_diff_names_deleted_file_from_old_side() {
        let raw = "diff --git a/gone.txt b/gone.txt\nindex 1..0 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let got = parse_commit_diff(raw);
        assert_eq!(got.files[0].path, "gone.txt");
    }

    /// Real repo: `log` lists newest-first with all fields, an unborn HEAD
    /// yields an empty list, and `commit_diff` returns the commit's patch.
    /// Skips when `git` is unavailable.
    #[test]
    fn log_and_commit_diff_read_real_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-git-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if run(&["init", "-q"]) {
            assert!(crate::git::log(&dir, 20).is_empty(), "unborn HEAD yields no commits");
            std::fs::write(dir.join("f.txt"), "one\n").unwrap();
            run(&["add", "f.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=Alice", "commit", "-qm", "first"]);
            std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
            run(&["add", "f.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=Bob", "commit", "-qm", "second"]);

            let got = crate::git::log(&dir, 20);
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].subject, "second", "newest first");
            assert_eq!(got[0].author, "Bob");
            assert!(!got[0].hash.is_empty() && !got[0].rel_time.is_empty());
            assert_eq!(got[1].subject, "first");

            let diff = crate::git::commit_diff(&dir, &got[0].hash).expect("show parses");
            assert_eq!(diff.files.len(), 1);
            assert_eq!(diff.files[0].path, "f.txt");
            assert!(
                diff.files[0]
                    .diff
                    .lines
                    .iter()
                    .any(|l| l.kind == crate::changes_diff::DiffLineKind::Added && l.text == "two")
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `revert` adds a new commit undoing the target — the file returns to
    /// its prior content and the log gains an entry. Skips when git is
    /// unavailable.
    #[test]
    fn revert_adds_a_reverting_commit() {
        let dir = std::env::temp_dir().join(format!("rixlcode-git-revert-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if run(&["init", "-q"]) {
            std::fs::write(dir.join("f.txt"), "one\n").unwrap();
            run(&["add", "f.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "first"]);
            std::fs::write(dir.join("f.txt"), "two\n").unwrap();
            run(&["add", "f.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "second"]);

            let sha = crate::git::log(&dir, 1)[0].hash.clone();
            crate::git::revert(&dir, &sha).unwrap();
            assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "one\n");
            let log = crate::git::log(&dir, 5);
            assert_eq!(log.len(), 3);
            assert!(log[0].subject.contains("Revert"), "newest commit is the revert");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
