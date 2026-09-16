//! Headless tests for "Copy resume command": the ⋯ menu item appears only
//! when the chat is bound to a backend thread AND the backend has a CLI
//! resume, and clicking it puts `cd <workdir> && <cmd>` on the clipboard.
//! Real backends stand in for codex/claude — no process ever spawns.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::backend::AgentBackend;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-resumecmd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
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

/// Bind the active chat to `thread_id` on `backend`.
fn bind_thread(
    ws: &Entity<Workspace>, cx: &mut VisualTestContext, backend: std::sync::Arc<dyn crate::backend::AgentBackend>, thread_id: &str,
) {
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            this.backend = backend;
            this.chats[this.active].thread_id = thread_id.to_string();
        });
    });
}

/// The ⋯ menu on the chat titlebar — opened by clicking the header button.
fn open_chat_menu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
    });
}

fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window).iter().filter_map(|s| s.label().map(|l| l.to_string())).collect()
}

/// Click the popup-menu item with `label` — panics when it isn't offered.
fn click_menu_item(window: &mut Window, label: &str, cx: &mut App) {
    let item = snapshots(window)
        .iter()
        .find(|s| s.label() == Some(label))
        .unwrap_or_else(|| panic!("menu should offer {label}"))
        .clone();
    let id = item.path().last().unwrap().clone();
    window.within("popup-menu").click(id, cx);
}

fn clipboard(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default())
}

fn codex() -> std::sync::Arc<dyn crate::backend::AgentBackend> {
    std::sync::Arc::new(crate::backend::CodexCliBackend::new(Vec::new()))
}

#[test]
fn backends_report_their_resume_shape() {
    assert_eq!(crate::backend::CodexCliBackend::new(Vec::new()).resume_command("t-1").as_deref(), Some("codex resume t-1"));
    assert_eq!(crate::backend::ClaudeCliBackend::new(Vec::new()).resume_command("t-1").as_deref(), Some("claude --resume t-1"));
    assert!(crate::backend::SimBackend.resume_command("t-1").is_none());
}

#[test]
fn no_resume_command_without_a_bound_thread() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "nothread");
    bind_thread(&ws, cx, codex(), "");
    cx.update(|_, cx| {
        assert!(ws.read(cx).resume_command().is_none(), "empty thread_id offers nothing");
    });
    open_chat_menu(cx);
    cx.update(|window, _cx| {
        let labels = menu_labels(window);
        assert!(!labels.iter().any(|l| l == "Copy resume command"), "no thread hides the item: {labels:?}");
    });
}

#[test]
fn no_resume_command_for_cli_less_backends() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "sim");
    bind_thread(&ws, cx, std::sync::Arc::new(crate::backend::SimBackend), "tid-1");
    cx.update(|_, cx| {
        assert!(ws.read(cx).resume_command().is_none(), "sim has no CLI resume");
    });
    open_chat_menu(cx);
    cx.update(|window, _cx| {
        let labels = menu_labels(window);
        assert!(!labels.iter().any(|l| l == "Copy resume command"), "sim hides the item: {labels:?}");
    });
}

#[test]
fn codex_resume_command_copies_cd_and_thread_id() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "codex");
    bind_thread(&ws, cx, codex(), "tid-1");
    let root = ws.read_with(cx, |this, _| this.project.root().to_path_buf());
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.copy_resume_command(cx));
    });
    assert_eq!(clipboard(cx), format!("cd '{}' && codex resume tid-1", root.display()));
    // The chat note confirms the copy.
    ws.read_with(cx, |this, _| {
        let last = this.chats[this.active].messages.last().expect("a note lands in the chat");
        assert!(last.markdown().contains("Copied:"), "note: {}", last.markdown());
        assert!(last.markdown().contains("codex resume tid-1"), "note: {}", last.markdown());
    });
}

#[test]
fn claude_resume_command_uses_dash_resume() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "claude");
    bind_thread(&ws, cx, std::sync::Arc::new(crate::backend::ClaudeCliBackend::new(Vec::new())), "sess-9");
    let root = ws.read_with(cx, |this, _| this.project.root().to_path_buf());
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.copy_resume_command(cx));
    });
    assert_eq!(clipboard(cx), format!("cd '{}' && claude --resume sess-9", root.display()));
}

#[test]
fn worktree_chat_resumes_in_the_worktree_dir() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "worktree");
    // An existing dir — `workdir_for` falls back to the root when it's gone.
    let dir = std::env::temp_dir().join(format!("rixlcode-resumecmd-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let chat = &mut this.chats[this.active];
            chat.worktree = true;
            chat.workdir = dir.to_string_lossy().into_owned();
        });
    });
    bind_thread(&ws, cx, codex(), "tid-7");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.copy_resume_command(cx));
    });
    assert_eq!(clipboard(cx), format!("cd '{}' && codex resume tid-7", dir.display()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn menu_item_copies_the_command() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "menu");
    bind_thread(&ws, cx, codex(), "tid-3");
    let root = ws.read_with(cx, |this, _| this.project.root().to_path_buf());
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Copy resume command"), "bound codex chat offers the item: {labels:?}");
        click_menu_item(window, "Copy resume command", cx);
    });
    assert_eq!(clipboard(cx), format!("cd '{}' && codex resume tid-3", root.display()));
}
