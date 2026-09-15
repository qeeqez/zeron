//! Headless tests for thread defaults: `default_model`,
//! `default_permissions` and `default_workspace` applied on `new_chat`,
//! per-thread stamps restored on `select_chat`, and the worktree cwd
//! plumbed into the backend spawn (via a recording fake — no real
//! subprocess).

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::workspace::Workspace;

/// A backend that records each turn's `TurnContext` instead of spawning —
/// the spawn-cwd assertion without a real subprocess.
struct RecordingBackend {
    ctxs: std::sync::Arc<parking_lot::Mutex<Vec<TurnContext>>>,
}

impl AgentBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn send(&self, _prompt: &str, _model: &str, _mode: &str, ctx: &TurnContext) -> ReplyStream {
        self.ctxs.lock().push(ctx.clone());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-td-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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

/// A temp git repo with one commit, opened as a project. Returns None
/// when git isn't installed.
fn temp_repo(name: &str) -> Option<crate::project::Project> {
    let dir = std::env::temp_dir().join(format!("rixlcode-td-repo-{name}-{}", std::process::id()));
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
        return None;
    }
    std::fs::write(dir.join("f.txt"), "hi").unwrap();
    assert!(git(&["add", "."]));
    assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]));
    Some(crate::project::Project::open(&dir))
}

#[test]
fn new_chat_applies_default_model_and_permissions() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "defaults");
    ws.update(cx, |this, cx| {
        let claude = this
            .providers
            .iter()
            .find(|p| p.kind == crate::providers::ProviderKind::ClaudeCli)
            .unwrap()
            .id
            .clone();
        this.default_model = crate::persist::DefaultModel {
            provider_instance_id: claude.clone(),
            model_id: "opus".into(),
        };
        this.default_permissions = Some(crate::backend::AccessMode::Supervised);
        this.new_chat(cx);
        let chat = &this.chats[this.active];
        assert_eq!(chat.provider, claude);
        assert_eq!(chat.model, "opus");
        assert_eq!(chat.access, Some(crate::backend::AccessMode::Supervised));
        // The workspace selection follows the new thread.
        assert_eq!(this.selected_provider, claude);
        assert_eq!(this.model.as_ref(), "opus");
        assert_eq!(this.access, crate::backend::AccessMode::Supervised);
    });
}

#[test]
fn existing_thread_keeps_its_stamp_when_defaults_change() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "stamps");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            let codex = this.selected_provider.clone();
            // Thread 1 is stamped with the current selection.
            this.new_chat(cx);
            let t1 = this.chats[this.active].id;
            assert_eq!(this.chats[this.active].provider, codex);

            // New defaults apply to thread 2 only.
            let claude = this
                .providers
                .iter()
                .find(|p| p.kind == crate::providers::ProviderKind::ClaudeCli)
                .unwrap()
                .id
                .clone();
            this.default_model = crate::persist::DefaultModel {
                provider_instance_id: claude.clone(),
                model_id: "haiku".into(),
            };
            this.default_permissions = Some(crate::backend::AccessMode::FullAccess);
            this.new_chat(cx);
            assert_eq!(this.chats[this.active].provider, claude);
            assert_eq!(this.chats[this.active].model, "haiku");
            assert_eq!(this.chats[this.active].access, Some(crate::backend::AccessMode::FullAccess));

            // Switching back restores thread 1's provider/model/access.
            let ix1 = this.chat_index(t1);
            this.select_chat(ix1.unwrap(), window, cx);
            assert_eq!(this.selected_provider, codex);
            assert_eq!(this.access, crate::backend::AccessMode::Auto);
            // And thread 2 still carries its own stamp.
            let t2 = this.chat_index(this.chats.iter().find(|c| c.id != t1 && c.provider == claude).unwrap().id);
            this.select_chat(t2.unwrap(), window, cx);
            assert_eq!(this.selected_provider, claude);
            assert_eq!(this.access, crate::backend::AccessMode::FullAccess);
        });
    });
}

#[test]
fn worktree_mode_creates_plumbs_cwd_and_cleans_up() {
    let Some(project) = temp_repo("wt") else { return };
    let root = project.root().to_path_buf();
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "wt");
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project = project.clone();
            this.default_workspace = crate::worktree::WorkspaceMode::Worktree;
            this.new_chat(cx);
            let chat = &this.chats[this.active];
            assert!(chat.worktree);
            let wt = std::path::PathBuf::from(&chat.workdir);
            assert!(wt.starts_with(root.join(".worktrees")));
            assert!(wt.join("f.txt").exists());

            // The backend spawn sees the worktree as its cwd.
            this.backend = std::sync::Arc::new(RecordingBackend { ctxs: ctxs.clone() });
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    let ctxs = ctxs.lock();
    assert_eq!(ctxs.len(), 1);
    let wt = ctxs[0].cwd.clone();
    assert!(wt.starts_with(root.join(".worktrees")));
    drop(ctxs);

    // Deleting the thread removes its worktree.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            let ix = this.active;
            this.delete_chat_now(ix, window, cx);
        });
    });
    assert!(!wt.exists());
    let _ = std::fs::remove_dir_all(&root);
}
