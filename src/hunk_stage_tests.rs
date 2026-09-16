//! Tests for hunk-level staging — `FileDiff::hunk_patch` slicing, the
//! `git apply --cached` ops against real temp repos, and the per-hunk
//! Stage/Unstage buttons in the expanded diff. Sibling file so `changes.rs`
//! stays under the SLOC cap; real-repo tests skip when git is unavailable.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;

    use crate::changes::GitOp;
    use crate::changes_diff::{DiffLineKind, diff_for_file, parse_diff};
    use crate::changes_ui_tests::{change, mount};
    use crate::git::{self, ChangeStatus};

    fn run(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// `git` stdout under `dir` — empty when the command fails.
    fn git_out(dir: &Path, args: &[&str]) -> String {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// A temp git repo with one commit of a 20-line file — edits at lines 6
    /// and 15 produce two hunks. Returns None when git isn't installed.
    fn two_hunk_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-hunk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if !run(&dir, &["init", "-q"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        assert!(run(&dir, &["config", "user.email", "t@t"]));
        assert!(run(&dir, &["config", "user.name", "t"]));
        let lines: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
        std::fs::write(dir.join("f.txt"), lines.join("\n") + "\n").unwrap();
        assert!(run(&dir, &["add", "f.txt"]));
        assert!(run(&dir, &["commit", "-qm", "init"]));
        let mut edited = lines;
        edited[5] = "CHANGED6".into();
        edited[14] = "CHANGED15".into();
        std::fs::write(dir.join("f.txt"), edited.join("\n") + "\n").unwrap();
        Some(dir)
    }

    /// The worktree diff for `f.txt` — two hunks, one per edited region.
    fn worktree_diff(dir: &Path) -> crate::changes_diff::FileDiff {
        let diff = diff_for_file(dir, &change("f.txt", ChangeStatus::Modified, 2, 2), false).unwrap();
        assert_eq!(diff.hunks.len(), 2, "two edited regions parse as two hunks");
        diff
    }

    #[test]
    fn hunk_patch_carves_header_plus_one_hunk() {
        let raw = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n@@ -8,2 +8,2 @@\n h\n-i\n+j\n";
        let diff = parse_diff(raw);
        assert_eq!(diff.hunks.len(), 2);
        assert_eq!(
            diff.hunk_patch(0).as_deref(),
            Some("diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n")
        );
        assert_eq!(
            diff.hunk_patch(1).as_deref(),
            Some("diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -8,2 +8,2 @@\n h\n-i\n+j\n")
        );
        assert!(diff.hunk_patch(2).is_none(), "no range past the last hunk");
    }

    /// A second file block's hunks render but get no patch range — staging
    /// one would apply the first file's header to the wrong path.
    #[test]
    fn second_file_block_hunks_have_no_patch() {
        let raw = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/g b/g\nindex 3..4 100644\n--- a/g\n+++ b/g\n@@ -1 +1 @@\n-x\n+y\n";
        let diff = parse_diff(raw);
        assert_eq!(diff.hunks.len(), 1, "only the first file's hunks are stageable");
        assert_eq!(diff.lines.iter().filter(|l| l.kind == DiffLineKind::Hunk).count(), 2, "both hunks still render");
    }

    /// Staging one hunk of a two-hunk file puts only that hunk in the index;
    /// the other stays unstaged. The acceptance criterion.
    #[test]
    fn stage_hunk_stages_only_that_hunk() {
        let Some(dir) = two_hunk_repo("stage") else { return };
        let diff = worktree_diff(&dir);
        let patch = diff.hunk_patch(0).unwrap();

        git::stage_hunk(&dir, "f.txt", &patch).unwrap();

        let staged = git_out(&dir, &["diff", "--cached"]);
        assert!(staged.contains("+CHANGED6"), "staged diff has the first hunk:\n{staged}");
        assert!(!staged.contains("CHANGED15"), "staged diff lacks the second hunk:\n{staged}");
        let unstaged = git_out(&dir, &["diff"]);
        assert!(unstaged.contains("+CHANGED15"), "worktree diff keeps the second hunk:\n{unstaged}");
        // "+CHANGED6" — the added-line marker; the bare string also appears
        // in the second hunk's @@ context text, so match the sign prefix.
        assert!(!unstaged.contains("+CHANGED6"), "worktree diff drops the staged hunk:\n{unstaged}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unstaging a staged hunk reverses it in the index — `git diff --cached`
    /// goes quiet while the worktree keeps both edits.
    #[test]
    fn unstage_hunk_restores_the_index() {
        let Some(dir) = two_hunk_repo("unstage") else { return };
        let diff = worktree_diff(&dir);
        git::stage_hunk(&dir, "f.txt", &diff.hunk_patch(0).unwrap()).unwrap();
        assert!(git_out(&dir, &["diff", "--cached"]).contains("CHANGED6"));
        // The staged row's diff is HEAD→index; reversing its hunk undoes it.
        let mut staged_change = change("f.txt", ChangeStatus::Modified, 1, 1);
        staged_change.staged = true;
        let staged_diff = diff_for_file(&dir, &staged_change, false).unwrap();
        git::unstage_hunk(&dir, "f.txt", &staged_diff.hunk_patch(0).unwrap()).unwrap();

        assert!(git_out(&dir, &["diff", "--cached"]).is_empty(), "index back at HEAD");
        let unstaged = git_out(&dir, &["diff"]);
        assert!(unstaged.contains("+CHANGED6") && unstaged.contains("+CHANGED15"), "both edits back in the worktree diff");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A patch that isn't a patch fails inside `git apply` — the op's Err
    /// lands as the panel's error note instead of staging anything.
    #[test]
    fn malformed_patch_lands_an_error_note() {
        let Some(dir) = two_hunk_repo("malformed") else { return };
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        cx.update(|_window, cx| {
            ws.update(cx, |this, cx| {
                this.project = crate::project::Project::open(&dir);
                this.run_git_op(GitOp::StageHunk { path: "f.txt".into(), patch: "not a patch\n".into() }, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_window, cx| {
            let note = ws.read(cx).git.note.as_ref().map(|(t, e)| (t.clone(), *e));
            let Some((text, is_error)) = note else { panic!("op landed a note") };
            assert!(is_error, "malformed patch is an error note: {text}");
        });
        assert!(git_out(&dir, &["diff", "--cached"]).is_empty(), "nothing staged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Untracked files have no index entry to patch against — whole-file
    /// `git add` only, so their hunk headers carry no Stage button. Added
    /// (staged-new) and deleted files can't be partially applied either.
    #[test]
    fn hunk_buttons_only_render_for_modified_files() {
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        let raw = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n";
        cx.update(|window, cx| {
            ws.update(cx, |this, cx| {
                let mut untracked = change("new.rs", ChangeStatus::Added, 1, 0);
                untracked.diff = Some(parse_diff(raw));
                let mut modified = change("edit.rs", ChangeStatus::Modified, 1, 1);
                modified.diff = Some(parse_diff(raw));
                let mut staged = change("staged.rs", ChangeStatus::Modified, 1, 1);
                staged.staged = true;
                staged.diff = Some(parse_diff(raw));
                this.changes = vec![untracked, modified, staged];
                this.changes_panel_open = true;
                cx.notify();
            });
            window.draw(cx).clear(cx);
            assert!(window.try_find(("hunk-stage", 0usize)).is_none(), "untracked file: no hunk button");
            // `next_line` counts every diff line across files — each fixture
            // diff is 3 rows, so the buttons land on ids 3 and 6.
            assert!(window.find(("hunk-stage", 3usize)).visible(), "modified file: Stage button");
            assert!(window.find(("hunk-stage", 6usize)).visible(), "staged file: Unstage button");
        });
    }

    /// End to end: expanding a modified row and clicking a hunk's Stage
    /// button runs `git apply --cached` — the refresh that lands shows the
    /// file partially staged, and the index holds only that hunk.
    #[test]
    fn hunk_stage_button_applies_the_hunk() {
        let Some(dir) = two_hunk_repo("button") else { return };
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        cx.update(|_window, cx| {
            ws.update(cx, |this, cx| {
                this.project = crate::project::Project::open(&dir);
                this.toggle_changes_panel(cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.click(("change-row", 0usize), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(ws.read(cx).changes[0].diff.is_some(), "diff expanded");
            window.click(("hunk-stage", 0usize), cx);
        });
        cx.run_until_parked();
        let staged = git_out(&dir, &["diff", "--cached"]);
        assert!(staged.contains("+CHANGED6"), "click staged the first hunk:\n{staged}");
        assert!(!staged.contains("CHANGED15"), "click left the second hunk unstaged:\n{staged}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
