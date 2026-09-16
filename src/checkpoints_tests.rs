//! Tests for per-turn checkpoints: real `git` in temp repos (skipped when
//! git is unavailable, same as `worktree_tests`), the file-copy fallback
//! for non-git dirs, checkpoint persistence, and the headless "Undo turn"
//! affordance on a completed turn.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::checkpoints::{Checkpoint, TurnCheckpoint, for_message, restore, snapshot};

    /// A temp git repo with one committed file — `None` when git isn't
    /// installed. `name` keeps parallel test dirs apart.
    fn temp_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-ckpt-{}-{name}", std::process::id()));
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
        std::fs::write(dir.join("base.txt"), "base").unwrap();
        std::fs::write(dir.join("gone.txt"), "gone").unwrap();
        assert!(git(&["add", "."]));
        assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
        Some(dir)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rixlcode-ckpt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn status(dir: &Path) -> String {
        crate::git::git(dir, &["status", "--porcelain"]).unwrap_or_default()
    }

    fn store() -> PathBuf {
        temp_dir("store")
    }

    #[test]
    fn git_checkpoint_roundtrips_dirty_worktree() {
        let Some(repo) = temp_repo("git") else { return };
        // Pre-turn state: a modified tracked file, a deleted one, and an
        // untracked file — the snapshot must capture all of it.
        std::fs::write(repo.join("base.txt"), "edited").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        std::fs::write(repo.join("new.txt"), "new").unwrap();
        let before = status(&repo);
        let Some(Checkpoint::Git(sha)) = snapshot(&repo, &store(), "c0-0") else { panic!("git snapshot") };

        // The turn edits files: modify, delete, create.
        std::fs::write(repo.join("base.txt"), "agent").unwrap();
        std::fs::remove_file(repo.join("new.txt")).unwrap();
        std::fs::write(repo.join("agent.txt"), "agent").unwrap();

        restore(&repo, &Checkpoint::Git(sha.clone())).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap(), "edited");
        assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "new");
        assert!(!repo.join("gone.txt").exists(), "pre-turn delete stays deleted");
        assert!(!repo.join("agent.txt").exists(), "turn-created file removed");
        assert_eq!(status(&repo), before, "worktree back to pre-turn state");
        // The user's real index is untouched: base.txt stays unstaged.
        assert!(status(&repo).contains(" M base.txt"));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn git_snapshot_leaves_no_status_noise() {
        let Some(repo) = temp_repo("clean") else { return };
        let before = status(&repo);
        assert!(snapshot(&repo, &store(), "c0-0").is_some());
        assert_eq!(status(&repo), before, "snapshot must not dirty status");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn git_restore_errors_on_missing_commit() {
        let Some(repo) = temp_repo("gone") else { return };
        let err = restore(&repo, &Checkpoint::Git("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into()));
        assert!(err.is_err(), "gc'd checkpoint must error, not silently pass");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn copy_checkpoint_roundtrips_plain_dir() {
        let dir = temp_dir("plain");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.txt"), "b").unwrap();
        let Some(Checkpoint::Copy(snap)) = snapshot(&dir, &store(), "c0-0") else { panic!("copy snapshot") };

        std::fs::write(dir.join("a.txt"), "agent").unwrap();
        std::fs::remove_file(dir.join("sub/b.txt")).unwrap();
        std::fs::write(dir.join("agent.txt"), "agent").unwrap();

        restore(&dir, &Checkpoint::Copy(snap)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(dir.join("sub/b.txt")).unwrap(), "b");
        assert!(!dir.join("agent.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_missing_dir_returns_none() {
        assert!(snapshot(Path::new("/definitely/gone"), &store(), "c0-0").is_none());
    }

    #[test]
    fn for_message_rejects_stale_index() {
        let mut chat = crate::model::Chat::new(0, "t");
        let at = std::time::SystemTime::now();
        chat.messages = std::rc::Rc::new(vec![crate::model::ChatMessage {
            role: crate::model::Role::User,
            kind: crate::model::MessageKind::Text("hi".into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            alternatives: vec![],
            at,
        }]);
        chat.checkpoints
            .push(TurnCheckpoint { ix: 0, at, checkpoint: Checkpoint::Copy(PathBuf::from("/x")) });
        assert!(for_message(&chat, 0).is_some());
        // A different message at the same index (post-/clear reuse) must
        // not match — its timestamp differs.
        chat.checkpoints.push(TurnCheckpoint {
            ix: 0,
            at: at - std::time::Duration::from_secs(60),
            checkpoint: Checkpoint::Copy(PathBuf::from("/y")),
        });
        assert_eq!(for_message(&chat, 0).unwrap().checkpoint, Checkpoint::Copy(PathBuf::from("/x")));
        assert!(for_message(&chat, 5).is_none());
    }

    #[test]
    fn checkpoints_survive_save_load() {
        let dir = temp_dir("persist");
        let mut chat = crate::model::Chat::new(0, "t");
        chat.messages = std::rc::Rc::new(vec![crate::model::ChatMessage {
            role: crate::model::Role::User,
            kind: crate::model::MessageKind::Text("hi".into()),
            rating: None,
            bookmarked: false,
            usage: None,
            alternatives: vec![],
            attachments: vec![],
            at: std::time::SystemTime::now(),
        }]);
        chat.checkpoints.push(TurnCheckpoint {
            ix: 0,
            at: chat.messages[0].at,
            checkpoint: Checkpoint::Git("abc123".into()),
        });
        crate::persist::save_chats(&dir, &[chat]);
        let mut next = 1;
        let loaded = crate::persist::load_chats(&dir, &mut next, false);
        assert_eq!(loaded.len(), 1);
        let cp = for_message(&loaded[0], 0).expect("checkpoint survives reload");
        assert_eq!(cp.checkpoint, Checkpoint::Git("abc123".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod ui {
    use std::path::PathBuf;

    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

    use crate::backend::{AgentBackend, AgentEvent, ReplyStream};
    use crate::workspace::Workspace;

    /// A backend that edits the turn's workdir like a real agent would —
    /// the file it writes is what "Undo turn" must remove.
    struct WriteBackend;

    impl AgentBackend for WriteBackend {
        fn name(&self) -> &'static str {
            "write"
        }

        fn send(&self, _prompt: &str, _model: &str, _mode: &str, ctx: &crate::backend::TurnContext) -> ReplyStream {
            std::fs::write(ctx.cwd.join("agent.txt"), "agent").unwrap();
            std::fs::write(ctx.cwd.join("base.txt"), "agent").unwrap();
            let (tx, events) = std::sync::mpsc::channel();
            let _ = tx.send(AgentEvent::TextDelta("done".into()));
            let _ = tx.send(AgentEvent::Done);
            ReplyStream {
                events,
                child: None,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        let dir = std::env::temp_dir().join(format!("rixlcode-ckpt-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: nextest runs each test in its own process.
        unsafe { std::env::set_var("HOME", &dir) };
        cx.update(gpui_kit::init);
        let mut ws = None;
        let (root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            ws = Some(view.clone());
            Root::new(view, window, cx)
        });
        let _ = root;
        (ws.unwrap(), cx)
    }

    /// A temp git repo the chat's turn runs in — keeps checkpoint refs out
    /// of the real project repo.
    fn temp_repo() -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("rixlcode-ckpt-ui-repo-{}", std::process::id()));
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
        std::fs::write(dir.join("base.txt"), "base").unwrap();
        assert!(git(&["add", "."]));
        assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
        Some(dir)
    }

    #[test]
    fn completed_turn_reverts_via_undo_button() {
        let Some(repo) = temp_repo() else { return };
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        cx.update(|window, cx| {
            ws.update(cx, |this, cx| {
                this.backend = std::sync::Arc::new(WriteBackend);
                this.chats[this.active].workdir = repo.to_string_lossy().into_owned();
                this.composer.update(cx, |composer, cx| composer.set_value("hi", window, cx));
                this.send(window, cx);
            });
        });
        // The pump thread is real — poll with real sleeps, not just clock
        // advances, so a loaded machine can't outrun the Done delivery.
        for _ in 0..200 {
            cx.executor().advance_clock(std::time::Duration::from_millis(50));
            cx.run_until_parked();
            if ws.read_with(cx, |ws, _| !ws.chats[0].running) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ws.read_with(cx, |ws, _| {
            let chat = &ws.chats[0];
            assert!(!chat.running, "turn finished");
            assert_eq!(chat.checkpoints.len(), 1, "turn recorded a checkpoint");
        });
        assert!(repo.join("agent.txt").exists(), "backend wrote during the turn");

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.hover(("msg", 0usize), cx);
            window.draw(cx).clear(cx);
            assert!(window.find(("revert", 0usize)).visible(), "Undo turn reveals on hover");
            window.click(("revert", 0usize), cx);
        });
        cx.run_until_parked();
        assert!(!repo.join("agent.txt").exists(), "revert removed the turn's new file");
        assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap(), "base");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
