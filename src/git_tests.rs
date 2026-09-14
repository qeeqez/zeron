//! Tests for `git` parsing — sibling file so `git.rs` stays under the SLOC
//! cap. All fixtures are canned command output; no real git runs in tests.

#[cfg(test)]
mod tests {
    use crate::git::{ChangeStatus, parse_numstat, parse_status};

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
}
