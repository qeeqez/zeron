//! Tests for custom instructions: the merge order (global then project),
//! project-file discovery (AGENTS.md / CLAUDE.md / .rixl/instructions.md),
//! persistence through `Settings`, and the `TurnContext` hand-off — a
//! recording fake captures what the backend would see, no subprocess.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::workspace::Workspace;

/// A backend that records each turn's `TurnContext` instead of spawning —
/// the instructions assertion without a real subprocess.
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

/// A fresh temp dir (HOME and project roots both live under it).
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-instr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mount a `Workspace` bound to `project` in a headless window, with `HOME`
/// redirected so settings/chats stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str, project: crate::project::Project) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let home = temp_dir(&format!("{name}-home"));
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &home) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::for_project(project, window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

#[test]
fn merge_orders_global_then_project() {
    assert_eq!(crate::instructions::merge("be terse", Some("use rust 2024")).as_deref(), Some("be terse\n\nuse rust 2024"));
}

#[test]
fn merge_skips_empty_sides() {
    assert_eq!(crate::instructions::merge("", Some("proj")).as_deref(), Some("proj"));
    assert_eq!(crate::instructions::merge("glob", None).as_deref(), Some("glob"));
    assert_eq!(crate::instructions::merge("  ", Some("  ")), None);
    assert_eq!(crate::instructions::merge("", None), None);
}

#[test]
fn project_file_reads_agents_md() {
    let root = temp_dir("agents");
    std::fs::write(root.join("AGENTS.md"), "project rules\n").unwrap();
    let (name, text) = crate::instructions::project_file(&root).unwrap();
    assert_eq!(name, "AGENTS.md");
    assert_eq!(text, "project rules");
}

#[test]
fn project_file_falls_back_in_order() {
    let root = temp_dir("fallback");
    std::fs::write(root.join("CLAUDE.md"), "claude rules").unwrap();
    std::fs::create_dir_all(root.join(".rixl")).unwrap();
    std::fs::write(root.join(".rixl/instructions.md"), "rixl rules").unwrap();
    // CLAUDE.md wins over .rixl/instructions.md; AGENTS.md wins over both.
    assert_eq!(crate::instructions::project_file(&root).unwrap().0, "CLAUDE.md");
    std::fs::write(root.join("AGENTS.md"), "agents rules").unwrap();
    assert_eq!(crate::instructions::project_file(&root).unwrap().0, "AGENTS.md");
}

#[test]
fn project_file_none_when_absent_or_empty() {
    let root = temp_dir("none");
    assert_eq!(crate::instructions::project_file(&root), None);
    std::fs::write(root.join("AGENTS.md"), "   \n").unwrap();
    assert_eq!(crate::instructions::project_file(&root), None, "whitespace-only file is ignored");
}

#[test]
fn prefixed_wraps_instructions() {
    let out = crate::instructions::prefixed("do the thing", Some("be terse"));
    assert_eq!(out, "<system_instructions>\nbe terse\n</system_instructions>\n\ndo the thing");
    assert_eq!(crate::instructions::prefixed("do the thing", None), "do the thing");
    assert_eq!(crate::instructions::prefixed("do the thing", Some("  ")), "do the thing");
}

#[test]
fn instructions_persist_through_settings() {
    let home = temp_dir("persist");
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("HOME", &home) };
    let mut s = crate::persist::load_settings();
    s.instructions = "always write tests".to_string();
    crate::persist::save_settings(&s);
    assert_eq!(crate::persist::load_settings().instructions, "always write tests");
    // And a field-less file still loads (serde default).
    let path = home.join(".rixl/rixlcode/settings.json");
    std::fs::write(&path, "{}").unwrap();
    assert_eq!(crate::persist::load_settings().instructions, "");
}

#[test]
fn turn_context_carries_merged_instructions() {
    let root = temp_dir("ctx");
    std::fs::write(root.join("AGENTS.md"), "project rules").unwrap();
    let project = crate::project::Project::open(&root);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "ctx", project);
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.instructions = "global rules".to_string();
            this.backend = std::sync::Arc::new(RecordingBackend { ctxs: ctxs.clone() });
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    let ctxs = ctxs.lock();
    assert_eq!(ctxs.len(), 1);
    assert_eq!(ctxs[0].instructions.as_deref(), Some("global rules\n\nproject rules"));
}

#[test]
fn turn_context_omits_instructions_when_empty() {
    let root = temp_dir("ctx-empty");
    let project = crate::project::Project::open(&root);
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "ctx-empty", project);
    let ctxs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.backend = std::sync::Arc::new(RecordingBackend { ctxs: ctxs.clone() });
            this.model = "m".into();
            this.composer.update(cx, |c, cx| c.set_value("hi", window, cx));
            this.send(window, cx);
        });
    });
    assert_eq!(ctxs.lock()[0].instructions, None);
}
