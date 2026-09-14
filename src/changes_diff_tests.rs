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
            std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
            run(&["add", "a.txt"]);
            run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
            std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
            std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();

            let modified = FileChange {
                path: "a.txt".into(),
                status: ChangeStatus::Modified,
                added: 2,
                deleted: 1,
                diff: None,
            };
            let diff = diff_for_file(&dir, &modified).unwrap();
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Removed && l.text == "two"));
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "TWO"));
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "three"));

            let untracked = FileChange {
                path: "new.txt".into(),
                status: ChangeStatus::Added,
                added: 1,
                deleted: 0,
                diff: None,
            };
            let diff = diff_for_file(&dir, &untracked).unwrap();
            assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Added && l.text == "fresh"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
