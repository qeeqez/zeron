//! Tests for the commit box's amend mode — `git::commit_amend` against real
//! temp repos (skipped when git is unavailable) and headless UI tests for
//! the Amend chip, same `TestAppContext::single()` pattern as
//! `changes_git_ui_tests.rs`.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;

    use crate::changes_ui_tests::mount;
    use crate::git::{self, BranchStatus, Commit};

    fn run(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// A temp git repo with one commit ("init") and a local identity.
    /// Returns None when git isn't installed.
    fn temp_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-amend-{name}-{}", std::process::id()));
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

    fn branch() -> BranchStatus {
        BranchStatus { name: "main".into(), upstream: None, ahead: 0, behind: 0 }
    }

    fn commit(subject: &str) -> Commit {
        Commit {
            hash: "abc1234".into(),
            subject: subject.into(),
            author: "t".into(),
            rel_time: "1h ago".into(),
            diff: None,
            diff_load: 0,
        }
    }

    /// Amending with a message rewrites HEAD's subject without adding a
    /// commit or touching the tree.
    #[test]
    fn commit_amend_replaces_the_message_and_keeps_the_tree() {
        let Some(dir) = temp_repo("msg") else { return };
        let tree_before = git::git(&dir, &["rev-parse", "HEAD^{tree}"]);

        git::commit_amend(&dir, Some("renamed subject")).unwrap();

        let subject = git::git(&dir, &["log", "-1", "--format=%s"]).unwrap_or_default();
        assert_eq!(subject.trim(), "renamed subject");
        let count = git::git(&dir, &["rev-list", "--count", "HEAD"]).unwrap_or_default();
        assert_eq!(count.trim(), "1", "amend rewrote HEAD instead of adding a commit");
        assert_eq!(git::git(&dir, &["rev-parse", "HEAD^{tree}"]), tree_before, "tree unchanged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Amending with `None` runs `--no-edit`: HEAD's subject survives and
    /// staged changes fold into the commit.
    #[test]
    fn commit_amend_without_message_keeps_the_subject() {
        let Some(dir) = temp_repo("noedit") else { return };
        std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
        git::stage(&dir, "f.txt").unwrap();

        git::commit_amend(&dir, None).unwrap();

        let subject = git::git(&dir, &["log", "-1", "--format=%s"]).unwrap_or_default();
        assert_eq!(subject.trim(), "init", "--no-edit kept the subject");
        let count = git::git(&dir, &["rev-list", "--count", "HEAD"]).unwrap_or_default();
        assert_eq!(count.trim(), "1");
        assert!(git::collect(&dir).is_empty(), "staged edit folded into HEAD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_commit_subject_reads_head() {
        let Some(dir) = temp_repo("subject") else { return };
        assert_eq!(git::last_commit_subject(&dir).as_deref(), Some("init"));
        let bare = std::env::temp_dir().join(format!("rixlcode-amend-norepo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        assert!(git::last_commit_subject(&bare).is_none(), "non-repo has no subject");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    /// The Amend chip renders once a commit exists; clicking it flips the
    /// commit button's label and the chip's checked state.
    #[test]
    fn amend_toggle_flips_the_button_label() {
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        cx.update(|window, cx| {
            ws.update(cx, |this, cx| {
                this.git.branch = Some(branch());
                this.git.commits = vec![commit("init")];
                this.changes_panel_open = true;
                cx.notify();
            });
            window.draw(cx).clear(cx);
            assert_eq!(window.find("commit-amend").checked(), Some(false), "amend starts off");
            assert_eq!(window.find("commit-button").label(), Some("Commit"));
            window.click("commit-amend", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(window.find("commit-amend").checked(), Some(true), "chip reads checked");
            assert_eq!(window.find("commit-button").label(), Some("Amend"), "button relabeled");
            assert!(ws.read(cx).git.amend);
        });
    }

    /// Amend on + a new message + Commit rewrites HEAD's subject — the
    /// refresh re-lists commits, the box clears, and the toggle resets.
    /// Skips when git is unavailable.
    #[test]
    fn amend_button_rewrites_head_with_new_message() {
        let Some(dir) = temp_repo("ui-amend") else { return };
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
            window.click("commit-amend", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(ws.read(cx).git.commit_input.read(cx).value().as_str(), "init", "turning amend on prefilled HEAD's subject");
            ws.update(cx, |this, cx| {
                this.git.commit_input.update(cx, |s, cx| s.set_value("amended subject", window, cx));
            });
            window.draw(cx).clear(cx);
            window.click("commit-button", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let git = &ws.read(cx).git;
            assert_eq!(git.commits.first().map(|c| c.subject.as_str()), Some("amended subject"));
            assert_eq!(git.commits.len(), 1, "amend rewrote HEAD, no new commit");
            assert_eq!(git.note.as_ref().map(|(t, e)| (t.as_str(), *e)), Some(("Amended", false)));
            assert!(git.commit_input.read(cx).value().is_empty(), "amend cleared the box");
            assert!(!git.amend, "a successful amend resets the toggle");
            assert_eq!(window.find("commit-button").label(), Some("Commit"), "label flipped back");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Amend on + an empty box still commits — `--no-edit` keeps HEAD's
    /// subject. Skips when git is unavailable.
    #[test]
    fn amend_with_empty_message_keeps_the_subject() {
        let Some(dir) = temp_repo("ui-noedit") else { return };
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
            window.click("commit-amend", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            ws.update(cx, |this, cx| {
                this.git.commit_input.update(cx, |s, cx| s.set_value("", window, cx));
            });
            window.draw(cx).clear(cx);
            window.click("commit-button", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let git = &ws.read(cx).git;
            assert_eq!(git.commits.first().map(|c| c.subject.as_str()), Some("init"), "subject kept");
            assert_eq!(git.commits.len(), 1);
            assert_eq!(git.note.as_ref().map(|(t, e)| (t.as_str(), *e)), Some(("Amended", false)));
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
