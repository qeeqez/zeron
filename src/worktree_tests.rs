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
