//! Tests for the per-thread worktree subsystem — real `git` against temp
//! repos (skipped when git is unavailable), plus pure path/mode logic.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::worktree::WorkspaceMode;

    /// A temp git repo with one commit — `worktree add` needs a HEAD.
    /// Returns None when git isn't installed.
    fn temp_repo(name: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-wt-{}-{name}", std::process::id()));
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
        std::fs::write(dir.join("f.txt"), "hi").unwrap();
        assert!(git(&["add", "."]));
        assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
        Some(dir)
    }

    #[test]
    fn workspace_mode_names_roundtrip() {
        for mode in WorkspaceMode::ALL {
            assert_eq!(WorkspaceMode::from_name(mode.name()), mode);
        }
        assert_eq!(WorkspaceMode::from_name("bogus"), WorkspaceMode::Checkout);
        assert_eq!(WorkspaceMode::from_name(""), WorkspaceMode::Checkout);
    }

    #[test]
    fn create_makes_a_detached_worktree_under_worktrees_dir() {
        let Some(root) = temp_repo("create") else { return };
        let project = crate::project::Project::open(&root);
        // `open` canonicalizes — compare against the project root, not the
        // raw temp path (/var → /private/var on macOS).
        let dir = crate::worktree::create(&project, 7).unwrap();
        assert_eq!(dir, project.worktrees_dir().join("thread-7"));
        assert!(dir.join("f.txt").exists());
        // The parent repo must not see the worktrees dir as untracked.
        let status = crate::git::git(project.root(), &["status", "--porcelain"]).unwrap_or_default();
        assert!(!status.contains(".worktrees"), "status: {status}");
        crate::worktree::remove(project.root(), &dir);
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_keeps_a_dirty_worktree() {
        let Some(root) = temp_repo("dirty") else { return };
        let project = crate::project::Project::open(&root);
        let dir = crate::worktree::create(&project, 3).unwrap();
        // Uncommitted work — an untracked file counts, same as git's own
        // `worktree remove` refusal.
        std::fs::write(dir.join("uncommitted.txt"), "wip").unwrap();
        let outcome = crate::worktree::remove(project.root(), &dir);
        assert!(matches!(outcome, crate::worktree::Removal::Kept(_)), "dirty checkout survives: {outcome:?}");
        assert!(dir.exists());
        // Still registered — `git worktree list` keeps pointing at it.
        let listed = crate::git::git(project.root(), &["worktree", "list"]).unwrap_or_default();
        assert!(listed.contains("thread-3"), "worktree list: {listed}");
        // Once clean, removal works.
        std::fs::remove_file(dir.join("uncommitted.txt")).unwrap();
        assert_eq!(crate::worktree::remove(project.root(), &dir), crate::worktree::Removal::Removed);
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_clean_marks_dirty_and_plain_dirs() {
        let Some(root) = temp_repo("clean") else { return };
        let project = crate::project::Project::open(&root);
        let dir = crate::worktree::create(&project, 4).unwrap();
        assert!(crate::worktree::is_clean(&dir), "fresh worktree is clean");
        std::fs::write(dir.join("wip.txt"), "x").unwrap();
        assert!(!crate::worktree::is_clean(&dir), "untracked file makes it dirty");
        // A plain dir (not a worktree at all) has no tracked state to lose.
        let plain = project.worktrees_dir().join("thread-leftover");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(crate::worktree::is_clean(&plain), "plain leftover dir counts as clean");
        crate::worktree::remove(project.root(), &dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_orphans_drops_clean_keeps_dirty_and_owned() {
        let Some(root) = temp_repo("prune") else { return };
        let project = crate::project::Project::open(&root);
        let clean_orphan = crate::worktree::create(&project, 10).unwrap();
        let dirty_orphan = crate::worktree::create(&project, 11).unwrap();
        std::fs::write(dirty_orphan.join("wip.txt"), "x").unwrap();
        let owned = crate::worktree::create(&project, 12).unwrap();
        // A plain leftover dir is not a git worktree — prune leaves it for
        // the settings list's manual Remove.
        let plain = project.worktrees_dir().join("thread-99");
        std::fs::create_dir_all(&plain).unwrap();
        let mut chat = crate::model::Chat::new(12, "live");
        chat.workdir = owned.to_string_lossy().into_owned();
        chat.worktree = true;

        let kept = crate::worktree::prune_orphans(project.root(), &[chat]);
        assert!(!clean_orphan.exists(), "clean orphan removed");
        assert!(dirty_orphan.exists(), "dirty orphan kept");
        assert_eq!(kept, vec![dirty_orphan.clone()]);
        assert!(owned.exists(), "a chat's worktree is never an orphan");
        assert!(plain.exists(), "plain dirs aren't auto-pruned");
        // The registry agrees: only the dirty orphan and the owned one.
        let listed = crate::git::git(project.root(), &["worktree", "list"]).unwrap_or_default();
        assert!(!listed.contains("thread-10"), "worktree list: {listed}");
        assert!(listed.contains("thread-11"));
        assert!(listed.contains("thread-12"));
        crate::worktree::remove(project.root(), &owned);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn create_fails_outside_a_repo() {
        let dir = std::env::temp_dir().join(format!("rixlcode-wt-{}-norepo", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project = crate::project::Project::open(&dir);
        assert!(crate::worktree::create(&project, 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workdir_for_falls_back_to_root() {
        let root = Path::new("/repo");
        let mut chat = crate::model::Chat::new(0, "t");
        // Empty workdir → project root.
        assert_eq!(crate::worktree::workdir_for(&chat, root), root);
        // A worktree path that no longer exists → project root.
        chat.workdir = "/definitely/gone".into();
        chat.worktree = true;
        assert_eq!(crate::worktree::workdir_for(&chat, root), root);
        // An existing worktree path → the worktree.
        chat.workdir = std::env::temp_dir().to_string_lossy().into_owned();
        assert_eq!(crate::worktree::workdir_for(&chat, root), std::env::temp_dir());
        // A non-worktree custom dir is used even if missing (never set in
        // practice — workdir only comes from create() or the root).
        chat.worktree = false;
        chat.workdir = "/definitely/gone".into();
        assert_eq!(crate::worktree::workdir_for(&chat, root), Path::new("/definitely/gone"));
    }
}
