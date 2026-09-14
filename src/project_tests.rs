//! Unit tests for `project::Project` — store layout, per-project isolation,
//! state round-trip and the legacy global-chats migration. Everything runs
//! under temp dirs; `sandbox_home` redirects `dirs_home` so project stores
//! never touch the real `~/.rixl/rixlcode`.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::model::Chat;
    use crate::project::{Project, ProjectState};

    /// Redirect `~` into a throwaway dir so project stores stay off the real
    /// profile. nextest runs each test in its own process, so no other
    /// thread can observe HOME mid-write.
    fn sandbox_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
        dir
    }

    /// A fresh project root under the system temp dir; `leaf` is the
    /// folder's own name so `Project::name` assertions stay meaningful.
    fn temp_root(leaf: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-proj-{}", std::process::id())).join(leaf);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn open_resolves_root_name_and_store() {
        sandbox_home();
        let root = temp_root("my app");
        let project = Project::open(&root);
        assert_eq!(project.root(), root.as_path());
        assert_eq!(project.name(), "my app");
        // Store dir lives under ~/.rixl/rixlcode/projects/ and is named
        // after the folder — spaces slugged, hash suffix for uniqueness.
        let id = project.dir().file_name().unwrap().to_str().unwrap();
        assert!(id.starts_with("my-app-"), "store id should slug the folder name, got {id}");
        assert!(project.dir().starts_with(crate::persist::dirs_home().join(".rixl/rixlcode/projects")));
        assert_eq!(project.chats_dir(), project.dir().join("chats"));
    }

    #[test]
    fn open_is_stable_and_collision_free() {
        sandbox_home();
        let a = temp_root("same");
        let b = temp_root("nested").join("same");
        std::fs::create_dir_all(&b).unwrap();
        let b = b.canonicalize().unwrap();
        // Same folder opened twice → same store; same-named folders in
        // different parents → different stores.
        assert_eq!(Project::open(&a).dir(), Project::open(&a).dir());
        assert_ne!(Project::open(&a).dir(), Project::open(&b).dir());
        // A file path opens its parent directory.
        let file = a.join("README.md");
        std::fs::write(&file, "hi").unwrap();
        assert_eq!(Project::open(&file).root(), a.as_path());
    }

    #[test]
    fn chats_are_isolated_per_project() {
        sandbox_home();
        let a = Project::open(temp_root("alpha"));
        let b = Project::open(temp_root("beta"));
        crate::persist::save_chats(&a.chats_dir(), &[Chat::new(0, "alpha chat")]);
        crate::persist::save_chats(&b.chats_dir(), &[Chat::new(0, "beta chat"), Chat::new(1, "beta two")]);

        let mut next_id = 0;
        let a_chats = crate::persist::load_chats(&a.chats_dir(), &mut next_id, true);
        let b_chats = crate::persist::load_chats(&b.chats_dir(), &mut next_id, true);
        assert_eq!(a_chats.len(), 1);
        assert_eq!(a_chats[0].title, "alpha chat");
        assert_eq!(b_chats.len(), 2);
        assert_eq!(b_chats[0].title, "beta chat");
    }

    #[test]
    fn project_state_roundtrips() {
        sandbox_home();
        let project = Project::open(temp_root("stateful"));
        assert_eq!(project.load_state().active_chat, 0, "missing state defaults to 0");
        project.save_state(&ProjectState { active_chat: 3 });
        assert_eq!(project.load_state().active_chat, 3);
        // The marker makes the hash-named dir self-describing.
        let marker: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(project.dir().join("project.json")).unwrap()).unwrap();
        assert_eq!(marker["name"], "stateful");
        assert_eq!(marker["root"], serde_json::to_value(project.root()).unwrap());
    }

    #[test]
    fn legacy_chats_migrate_into_launch_project() {
        let home = sandbox_home();
        let legacy = home.join(".rixl/rixlcode/chats");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("0.json"), r#"{"v":1,"title":"old chat","messages":[]}"#).unwrap();
        std::fs::write(legacy.join("1.json"), r#"{"v":1,"title":"older","messages":[]}"#).unwrap();

        let project = Project::open(temp_root("migrated"));
        project.migrate_legacy_chats(1);

        let mut next_id = 0;
        let chats = crate::persist::load_chats(&project.chats_dir(), &mut next_id, true);
        assert_eq!(chats.len(), 2, "legacy chats must land in the project store");
        assert_eq!(chats[0].title, "old chat");
        assert_eq!(project.load_state().active_chat, 1, "legacy active_chat seeds project state");
        assert!(!legacy.exists(), "emptied legacy dir is removed");
    }

    #[test]
    fn migration_never_clobbers_existing_project_chats() {
        let home = sandbox_home();
        let legacy = home.join(".rixl/rixlcode/chats");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("0.json"), r#"{"v":1,"title":"legacy","messages":[]}"#).unwrap();

        let project = Project::open(temp_root("occupied"));
        crate::persist::save_chats(&project.chats_dir(), &[Chat::new(0, "already here")]);
        project.migrate_legacy_chats(0);

        let mut next_id = 0;
        let chats = crate::persist::load_chats(&project.chats_dir(), &mut next_id, true);
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].title, "already here", "existing project chats must win");
        assert!(legacy.join("0.json").exists(), "unmigrated legacy file stays put");
    }

    #[test]
    fn migration_keeps_source_when_move_fails() {
        let home = sandbox_home();
        let legacy = home.join(".rixl/rixlcode/chats");
        std::fs::create_dir_all(&legacy).unwrap();
        let source = legacy.join("0.json");
        std::fs::write(&source, r#"{"v":1,"title":"old chat","messages":[]}"#).unwrap();

        // A file where the chats dir belongs makes every move fail — rename
        // can't create the target, and the copy fallback can't either. The
        // legacy source must survive so the next launch can retry.
        let project = Project::open(temp_root("blocked"));
        std::fs::create_dir_all(project.dir()).unwrap();
        std::fs::write(project.chats_dir(), "not a directory").unwrap();
        project.migrate_legacy_chats(0);

        assert!(source.exists(), "a failed move must not delete the legacy chat");
        assert_eq!(std::fs::read_to_string(&source).unwrap(), r#"{"v":1,"title":"old chat","messages":[]}"#);
    }

    #[test]
    fn partial_migration_resumes_on_next_launch() {
        let home = sandbox_home();
        let legacy = home.join(".rixl/rixlcode/chats");
        std::fs::create_dir_all(&legacy).unwrap();
        let moved = r#"{"v":1,"title":"moved","messages":[]}"#;
        let stranded = r#"{"v":1,"title":"stranded","messages":[]}"#;
        std::fs::write(legacy.join("0.json"), moved).unwrap();
        std::fs::write(legacy.join("1.json"), stranded).unwrap();

        // Simulate a run that moved 0.json then died before 1.json: the
        // target holds the moved file and the in-progress marker survives.
        let project = Project::open(temp_root("partial"));
        std::fs::create_dir_all(project.chats_dir()).unwrap();
        std::fs::rename(legacy.join("0.json"), project.chats_dir().join("0.json")).unwrap();
        std::fs::write(project.dir().join("legacy-migration"), "").unwrap();

        // Next launch: the occupied target must not strand 1.json — the
        // marker says this run is a resume, not a foreign history.
        project.migrate_legacy_chats(0);

        let mut next_id = 0;
        let chats = crate::persist::load_chats(&project.chats_dir(), &mut next_id, true);
        assert_eq!(chats.len(), 2, "the stranded legacy chat must be retried");
        assert_eq!(chats[0].title, "moved");
        assert_eq!(chats[1].title, "stranded");
        assert!(!legacy.exists(), "emptied legacy dir is removed");
        assert!(!project.dir().join("legacy-migration").exists(), "marker clears once migration completes");
    }

    #[test]
    fn resumed_migration_never_overwrites_occupied_slots() {
        let home = sandbox_home();
        let legacy = home.join(".rixl/rixlcode/chats");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("0.json"), r#"{"v":1,"title":"legacy","messages":[]}"#).unwrap();

        // A resume where slot 0 was taken by a different chat mid-run —
        // the legacy file must land on a free index, not clobber it.
        let project = Project::open(temp_root("collision"));
        std::fs::create_dir_all(project.chats_dir()).unwrap();
        std::fs::write(project.chats_dir().join("0.json"), r#"{"v":1,"title":"other","messages":[]}"#).unwrap();
        std::fs::write(project.dir().join("legacy-migration"), "").unwrap();

        project.migrate_legacy_chats(0);

        let mut next_id = 0;
        let chats = crate::persist::load_chats(&project.chats_dir(), &mut next_id, true);
        assert_eq!(chats.len(), 2, "both chats must survive the collision");
        assert_eq!(chats[0].title, "other", "the occupied slot keeps its chat");
        assert_eq!(chats[1].title, "legacy", "the legacy chat takes a free index");
        assert!(!legacy.exists());
    }

    #[test]
    fn launch_resolves_once_for_all_windows() {
        sandbox_home();
        let original = std::env::current_dir().unwrap().canonicalize().unwrap();
        let first = Project::launch();
        assert_eq!(first.root(), original.as_path());

        // `enter` re-roots the process; a second Workspace (New Window) must
        // reuse the resolved project instead of re-reading args against the
        // new cwd — a relative dir arg would otherwise nest (`repo/repo`).
        let elsewhere = temp_root("elsewhere");
        std::env::set_current_dir(&elsewhere).unwrap();
        let second = Project::launch();
        assert_eq!(second.root(), original.as_path(), "New Window must reuse the launch project");
        assert_eq!(second.dir(), first.dir());
    }

    #[test]
    fn open_missing_path_falls_back_to_cwd() {
        sandbox_home();
        let project = Project::open(Path::new("/definitely/not/a/real/dir"));
        assert_eq!(project.root(), std::env::current_dir().unwrap().as_path());
    }
}
