//! Tests for the merge-conflict plumbing — real `git` against temp repos
//! with an actual merge conflict (skipped when git is unavailable), plus
//! `parse_names` fixtures. The headless section tests live in
//! `changes_conflicts_ui_tests.rs`.

#[cfg(test)]
mod tests {
    use crate::changes_conflicts::{self, ConflictSide};
    use crate::git;
    use crate::git_parse::parse_names;
    use std::path::{Path, PathBuf};

    fn run(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// A temp repo mid-merge-conflict on `f.txt`: `main` wrote "ours", the
    /// merged `side` branch wrote "theirs". Returns None when git isn't
    /// installed.
    fn conflicted_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-conflicts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if !run(&dir, &["init", "-q", "-b", "main"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        assert!(run(&dir, &["config", "user.email", "t@t"]));
        assert!(run(&dir, &["config", "user.name", "t"]));
        std::fs::write(dir.join("f.txt"), "base\n").unwrap();
        assert!(run(&dir, &["add", "f.txt"]));
        assert!(run(&dir, &["commit", "-qm", "init"]));
        assert!(run(&dir, &["checkout", "-qb", "side"]));
        std::fs::write(dir.join("f.txt"), "theirs\n").unwrap();
        assert!(run(&dir, &["commit", "-qam", "side"]));
        assert!(run(&dir, &["checkout", "-q", "main"]));
        std::fs::write(dir.join("f.txt"), "ours\n").unwrap();
        assert!(run(&dir, &["commit", "-qam", "main"]));
        assert!(!run(&dir, &["merge", "side"]), "the fixture merge must conflict");
        Some(dir)
    }

    #[test]
    fn parse_names_splits_nul_fields() {
        assert_eq!(parse_names("a.txt\0b c.txt\0"), ["a.txt", "b c.txt"]);
        assert!(parse_names("").is_empty());
    }

    #[test]
    fn conflicted_files_lists_unmerged_paths() {
        let Some(dir) = conflicted_repo("list") else { return };
        assert_eq!(changes_conflicts::conflicted_files(&dir), ["f.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conflicted_files_is_empty_on_a_clean_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-conflicts-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if !run(&dir, &["init", "-q"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert!(changes_conflicts::conflicted_files(&dir).is_empty());
        // A non-repo dir reports empty too — `git diff` fails there.
        let plain = std::env::temp_dir().join(format!("rixlcode-conflicts-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        assert!(changes_conflicts::conflicted_files(&plain).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&plain);
    }

    #[test]
    fn resolve_ours_keeps_our_version_and_stages() {
        let Some(dir) = conflicted_repo("ours") else { return };
        assert!(changes_conflicts::resolve_conflict(&dir, "f.txt", ConflictSide::Ours).is_ok());
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "ours\n");
        assert!(changes_conflicts::conflicted_files(&dir).is_empty(), "staged resolution leaves the unmerged list");
        // `--ours` restores the HEAD content, so porcelain is quiet — the
        // index's unmerged entries are the real "still conflicted" signal.
        let unmerged = git::git(&dir, &["ls-files", "-u"]).unwrap_or_default();
        assert!(unmerged.is_empty(), "no unmerged index entries: {unmerged:?}");
    }

    #[test]
    fn resolve_theirs_keeps_their_version() {
        let Some(dir) = conflicted_repo("theirs") else { return };
        assert!(changes_conflicts::resolve_conflict(&dir, "f.txt", ConflictSide::Theirs).is_ok());
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "theirs\n");
        assert!(changes_conflicts::conflicted_files(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The "Mark resolved" path: the user edits the markers away by hand,
    /// then `git add` stages the result and clears the conflict.
    #[test]
    fn stage_marks_a_hand_edited_conflict_resolved() {
        let Some(dir) = conflicted_repo("mark") else { return };
        std::fs::write(dir.join("f.txt"), "merged\n").unwrap();
        assert!(git::stage(&dir, "f.txt").is_ok());
        assert!(changes_conflicts::conflicted_files(&dir).is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "merged\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
