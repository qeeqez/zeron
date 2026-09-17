//! Headless tests for the message row's right-click menu: the copy
//! variants group on top; "Copy as Markdown" writes the raw source and
//! "Copy Code" the fenced block's contents.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-menu-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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

/// The menu entity is built in a deferred callback after the right-click's
/// update, so the labels are read one update later.
fn menu_labels(window: &mut gpui_kit::Window) -> Vec<String> {
    let mut labels: Vec<String> = snapshots(window)
        .iter()
        .filter(|s| s.role() == Some(A11yRole::MenuItem))
        .filter_map(|s| s.label().map(str::to_string))
        .collect();
    labels.sort();
    labels
}

#[test]
fn context_menu_lists_copy_variants() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| this.push_note("**bold** reply".into(), cx));
    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "right-click should open the message menu");
        assert_eq!(
            menu_labels(window),
            ["Bookmark", "Copy", "Copy as Markdown", "Quote", "Regenerate with model", "Retry", "View raw"],
            "menu should list the copy variants — Fork from here stays hidden on the last message"
        );
        window.within("popup-menu").click(1usize, cx); // Copy as Markdown
    });
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    assert_eq!(clip, "**bold** reply", "Copy as Markdown should write the raw source");
}

/// A message with a fenced block also lists "Copy Code", which writes the
/// block contents without the fences.
#[test]
fn context_menu_copy_code_writes_block_contents() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.update(cx, |this, cx| this.push_note("try:\n\n```rust\nfn main() {}\n```".into(), cx));
    cx.update(|window, cx| {
        window.right_click(("msg", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(
            menu_labels(window),
            [
                "Bookmark",
                "Copy",
                "Copy Code",
                "Copy as Markdown",
                "Quote",
                "Regenerate with model",
                "Retry",
                "View raw",
            ],
            "Copy Code should join the copy group"
        );
        window.within("popup-menu").click(2usize, cx); // Copy Code
    });
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    assert_eq!(clip, "fn main() {}", "Copy Code should write the block contents");
}
