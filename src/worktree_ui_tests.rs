//! Headless UI tests for the worktree affordances: the titlebar badge, the
//! sidebar row glyph, and the ⋯ menu's reveal/open items. The `open_in`
//! recorder seam asserts the exact argv — nothing spawns.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, VisualTestContext, Window};

use crate::open_in::{ISSUED, PreferredEditor, open_command, reveal_command};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-wtui-test-{}", std::process::id()));
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

/// An existing dir to stand in for the chat's worktree — `workdir_for`
/// falls back to the project root when the path is gone.
fn fake_worktree(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-wtui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mark the active chat as a worktree thread rooted at `dir`.
fn make_worktree(ws: &Entity<Workspace>, cx: &mut VisualTestContext, dir: &std::path::Path) -> u64 {
    cx.update(|_, cx| {
        ws.update(cx, |this, _| {
            let chat = &mut this.chats[this.active];
            chat.worktree = true;
            chat.workdir = dir.to_string_lossy().into_owned();
            chat.id
        })
    })
}

/// Wait for a background `run_open_command` task to record `n` commands.
fn until_issued(cx: &mut VisualTestContext, n: usize) {
    for _ in 0..200 {
        cx.run_until_parked();
        if ISSUED.lock().len() >= n {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("expected {n} issued commands, got {:?}", ISSUED.lock());
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

fn menu_labels(window: &Window) -> Vec<String> {
    snapshots(window).iter().filter_map(|s| s.label().map(|l| l.to_string())).collect()
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

#[test]
fn worktree_chat_shows_header_badge_and_row_glyph() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let dir = fake_worktree("badge");
    let chat_id = make_worktree(&ws, cx, &dir);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("worktree-badge").visible(), "worktree chat shows the titlebar badge");
        assert!(window.find(("worktree-glyph", chat_id)).visible(), "worktree chat shows the row glyph");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plain_chat_hides_the_worktree_affordances() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let chat_id = cx.update(|_, cx| ws.read(cx).chats[0].id);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("worktree-badge").is_none(), "no badge without a worktree");
        assert!(window.try_find(("worktree-glyph", chat_id)).is_none(), "no row glyph without a worktree");
    });
}

#[test]
fn chat_menu_lists_worktree_items_only_for_worktree_chats() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Plain chat: neither item is offered.
    open_chat_menu(cx);
    cx.update(|window, _cx| {
        let labels = menu_labels(window);
        assert!(!labels.iter().any(|l| l == "Reveal Worktree in Finder"), "plain chat hides reveal: {labels:?}");
        assert!(!labels.iter().any(|l| l.starts_with("Open Worktree in")), "plain chat hides open: {labels:?}");
    });
    cx.update(|window, cx| window.press("escape", cx));

    // Worktree chat: both items appear.
    let dir = fake_worktree("menu");
    make_worktree(&ws, cx, &dir);
    open_chat_menu(cx);
    cx.update(|window, _cx| {
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Reveal Worktree in Finder"), "worktree chat offers reveal: {labels:?}");
        assert!(labels.iter().any(|l| l.starts_with("Open Worktree in")), "worktree chat offers open: {labels:?}");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reveal_worktree_issues_open_dash_r() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let dir = fake_worktree("reveal");
    make_worktree(&ws, cx, &dir);
    open_chat_menu(cx);
    cx.update(|window, cx| click_menu_item(window, "Reveal Worktree in Finder", cx));
    until_issued(cx, 1);
    assert_eq!(ISSUED.lock().as_slice(), &[reveal_command(&dir)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_worktree_in_editor_issues_open_dash_a() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let dir = fake_worktree("editor");
    make_worktree(&ws, cx, &dir);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.set_preferred_editor(PreferredEditor::VsCode, cx));
    });
    open_chat_menu(cx);
    cx.update(|window, cx| click_menu_item(window, "Open Worktree in VS Code", cx));
    until_issued(cx, 1);
    let expected = open_command(PreferredEditor::VsCode, &dir).expect("VsCode builds a command");
    assert_eq!(ISSUED.lock().as_slice(), &[expected]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ask_editor_expands_the_worktree_picker() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    let dir = fake_worktree("ask");
    make_worktree(&ws, cx, &dir);
    // Default `preferred_editor` is Ask — the item becomes a submenu.
    open_chat_menu(cx);
    cx.update(|window, cx| {
        let item = snapshots(window)
            .iter()
            .find(|s| s.label() == Some("Open Worktree in Editor"))
            .unwrap_or_else(|| panic!("Ask should offer the editor submenu"))
            .clone();
        let id = item.path().last().unwrap().clone();
        window.within("popup-menu").hover(id, cx);
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        for editor in PreferredEditor::CHOICES {
            assert!(labels.iter().any(|l| l == editor.label()), "submenu offers {}: {labels:?}", editor.label());
        }
        window.within("submenu").click(0usize, cx); // VS Code
    });
    until_issued(cx, 1);
    let expected = open_command(PreferredEditor::VsCode, &dir).expect("VsCode builds a command");
    assert_eq!(ISSUED.lock().as_slice(), &[expected]);
    let _ = std::fs::remove_dir_all(&dir);
}
