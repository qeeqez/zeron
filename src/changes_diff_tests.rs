//! Tests for `changes_diff` — unified-diff parsing fixtures plus one real-repo
//! `diff_for_file` round-trip. Sibling file so `changes_diff.rs` stays under
//! the SLOC cap.

#[cfg(test)]
mod tests {
    use crate::changes_diff::{DiffLineKind, FileDiff, diff_for_file, parse_diff};
    use crate::git::{ChangeStatus, FileChange};

    fn kinds(diff: &FileDiff) -> Vec<DiffLineKind> {
        diff.lines.iter().map(|l| l.kind).collect()
    }

    /// Run `git` in `dir`; true on exit 0. Real-repo tests skip when it fails.
    fn git_ok(dir: &std::path::Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn change(path: &str, status: ChangeStatus, added: u32, deleted: u32) -> FileChange {
        FileChange {
            path: path.into(),
            source: None,
            status,
            added,
            deleted,
            staged: false,
            diff: None,
            diff_load: 0,
        }
    }

    #[test]
    fn parses_hunks_with_line_numbers() {
        let raw = "diff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,3 +10,4 @@ fn main() {\n ctx\n-old();\n+new();\n+extra();\n tail\n";
        let diff = parse_diff(raw);
        assert_eq!(
            kinds(&diff),
            [
                DiffLineKind::Hunk,
                DiffLineKind::Context,
                DiffLineKind::Removed,
                DiffLineKind::Added,
                DiffLineKind::Added,
                DiffLineKind::Context,
            ]
        );
        assert_eq!((diff.lines[1].old, diff.lines[1].new), (Some(10), Some(10)));
        assert_eq!((diff.lines[2].old, diff.lines[2].new), (Some(11), None));
        assert_eq!((diff.lines[3].old, diff.lines[3].new), (None, Some(11)));
        assert_eq!((diff.lines[4].old, diff.lines[4].new), (None, Some(12)));
        assert_eq!((diff.lines[5].old, diff.lines[5].new), (Some(12), Some(13)));
        assert_eq!(diff.lines[3].text, "new();", "marker is stripped");
        assert!(!diff.truncated);
    }

    #[test]
    fn multiple_hunks_restart_numbering() {
        let raw = "@@ -1,1 +1,1 @@\n-a\n+b\n@@ -50,1 +50,2 @@\n ctx\n+c\n";
        let diff = parse_diff(raw);
        assert_eq!(diff.lines.len(), 6);
        assert_eq!((diff.lines[4].old, diff.lines[4].new), (Some(50), Some(50)));
        assert_eq!((diff.lines[5].old, diff.lines[5].new), (None, Some(51)));
    }

    #[test]
    fn no_newline_marker_and_empty_context_survive() {
        let raw = "@@ -1,2 +1,2 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n";
        let diff = parse_diff(raw);
        assert_eq!(
            kinds(&diff),
            [
                DiffLineKind::Hunk,
                DiffLineKind::Removed,
                DiffLineKind::Context,
                DiffLineKind::Added,
                DiffLineKind::Context
            ]
        );
        assert_eq!(diff.lines[2].text, "\\ No newline at end of file");
    }

    #[test]
    fn headers_before_first_hunk_are_skipped() {
        let raw = "diff --git a/f b/f\nnew file mode 100644\nindex 000..111\n--- /dev/null\n+++ b/f\n@@ -0,0 +1,1 @@\n+hi\n";
        let diff = parse_diff(raw);
        assert_eq!(kinds(&diff), [DiffLineKind::Hunk, DiffLineKind::Added]);
    }

    #[test]
    fn empty_and_header_only_input_yield_no_lines() {
        assert!(parse_diff("").lines.is_empty());
        assert!(parse_diff("diff --git a/f b/f\nindex 1..2 100644\n").lines.is_empty());
    }

    #[test]
    fn oversized_diff_is_truncated() {
        let mut raw = String::from("@@ -1,1 +1,1 @@\n");
        for _ in 0..600 {
            raw.push_str("+line\n");
        }
        let diff = parse_diff(&raw);
        assert_eq!(diff.lines.len(), 400);
        assert!(diff.truncated);
    }

    /// End-to-end against a real repo: a modified tracked file produces added
    /// and removed lines, and an untracked file diffs against /dev/null.
    /// Skips when `git` is unavailable.
    #[test]
    fn diff_for_file_reads_real_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-diff-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git_ok(&dir, args);
        if run(&["init", "-q"]) {
            std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
            run(&["add", "a.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
            std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
            std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();

            let diff = diff_for_file(&dir, &change("a.txt", ChangeStatus::Modified, 2, 1)).unwrap();
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Removed && l.text == "two"));
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "TWO"));
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "three"));

            let diff = diff_for_file(&dir, &change("new.txt", ChangeStatus::Added, 1, 0)).unwrap();
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "fresh"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A staged rename must diff HEAD→worktree across both names — with only
    /// the destination path, `git diff HEAD` reports it as an all-added new
    /// file and the removed lines never show. Skips when `git` is unavailable.
    #[test]
    fn diff_for_rename_shows_delta_not_all_added() {
        let dir = std::env::temp_dir().join(format!("rixlcode-diff-rename-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git_ok(&dir, args);
        if run(&["init", "-q"]) {
            std::fs::write(dir.join("old.txt"), "one\ntwo\nthree\n").unwrap();
            run(&["add", "old.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
            run(&["mv", "old.txt", "new.txt"]);
            std::fs::write(dir.join("new.txt"), "one\nTWO\nthree\n").unwrap();

            let mut renamed = change("new.txt", ChangeStatus::Renamed, 1, 1);
            renamed.source = Some("old.txt".into());
            let diff = diff_for_file(&dir, &renamed).unwrap();
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Removed && l.text == "two"));
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "TWO"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On an unborn HEAD, a staged-then-edited file diffs once against the
    /// empty tree — the net worktree content — instead of concatenating the
    /// empty→index and index→worktree patches (whose second set of headers
    /// `parse_diff` would read as content). Skips when `git` is unavailable.
    #[test]
    fn diff_for_unborn_head_shows_net_worktree() {
        let dir = std::env::temp_dir().join(format!("rixlcode-diff-unborn-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git_ok(&dir, args);
        if run(&["init", "-q"]) {
            std::fs::write(dir.join("f.txt"), "a\nb\n").unwrap();
            run(&["add", "f.txt"]);
            std::fs::write(dir.join("f.txt"), "a\nB\nc\n").unwrap();

            let diff = diff_for_file(&dir, &change("f.txt", ChangeStatus::Added, 3, 0)).unwrap();
            let added: Vec<&str> = diff.lines.iter().filter(|l| l.kind == DiffLineKind::Added).map(|l| l.text.as_str()).collect();
            assert_eq!(added, ["a", "B", "c"], "net worktree content, no header junk");
            assert!(diff.lines.iter().all(|l| l.kind != DiffLineKind::Removed));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// In a SHA-256 repo the empty tree is `6ef19b…`, not the SHA-1
    /// `4b825dc…` — the unborn-HEAD fallback must derive it from the repo's
    /// object format or `git diff` exits 128. Skips when `git` is unavailable
    /// or too old for `--object-format=sha256`.
    #[test]
    fn diff_for_unborn_head_works_in_sha256_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-diff-sha256-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git_ok(&dir, args);
        if run(&["init", "-q", "--object-format=sha256"]) {
            assert_eq!(
                crate::git::empty_tree_id(&dir).as_deref(),
                Some("6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"),
                "empty tree derived in the repo's object format"
            );
            std::fs::write(dir.join("f.txt"), "a\nb\n").unwrap();
            run(&["add", "f.txt"]);
            std::fs::write(dir.join("f.txt"), "a\nB\nc\n").unwrap();

            let diff = diff_for_file(&dir, &change("f.txt", ChangeStatus::Added, 3, 0)).unwrap();
            let added: Vec<&str> = diff.lines.iter().filter(|l| l.kind == DiffLineKind::Added).map(|l| l.text.as_str()).collect();
            assert_eq!(added, ["a", "B", "c"], "net worktree content via the sha256 empty tree");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A diff bigger than the byte cap is cut at the subprocess, not buffered
    /// whole — the row cap alone can't flag it, so `truncated` comes from the
    /// byte limit. Skips when `git` is unavailable.
    #[test]
    fn oversized_diff_is_capped_at_the_pipe() {
        let dir = std::env::temp_dir().join(format!("rixlcode-diff-cap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git_ok(&dir, args);
        if run(&["init", "-q"]) {
            std::fs::write(dir.join("big.txt"), "x\n").unwrap();
            run(&["add", "big.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
            // One ~600KB line — over the 512KB byte cap in a single row.
            std::fs::write(dir.join("big.txt"), "a".repeat(600 * 1024)).unwrap();

            let diff = diff_for_file(&dir, &change("big.txt", ChangeStatus::Modified, 1, 1)).unwrap();
            assert!(diff.truncated, "byte cap marks the diff truncated");
            assert!(diff.lines.len() <= 400);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
