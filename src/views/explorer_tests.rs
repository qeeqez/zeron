//! Explorer tests: `build_file_tree`/`mention_text` units plus headless
//! runs through the real sidebar — tab switch, dir expand/collapse, and a
//! file click landing an @-mention in the composer. Narrow imports on
//! purpose (see `composer_testutil`).
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, KeyBinding, TestAppContext};

use crate::composer_testutil::{composer_value, open_workspace};
use crate::files::{DirNode, build_file_tree};
use crate::views::explorer::mention_text;
use crate::views::sidebar::SidebarTab;
use crate::workspace::Workspace;

fn ss(s: &str) -> gpui_kit::SharedString {
    s.into()
}

/// A canned tree: two top-level dirs (one nested), one root file.
fn canned_files() -> Vec<gpui_kit::SharedString> {
    ["src/lib.rs", "src/main.rs", "src/ui/panel.rs", "docs/guide.md", "README.md"]
        .iter()
        .map(|s| ss(s))
        .collect()
}

#[test]
fn tree_groups_files_into_nested_sorted_dirs() {
    let tree = build_file_tree(&canned_files());
    assert_eq!(tree.files, vec![ss("README.md")], "root file stays at the top level");
    let names: Vec<_> = tree.dirs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["docs", "src"], "dirs sort by name");
    assert_eq!(tree.dirs[0].path.as_str(), "docs");
    assert_eq!(tree.dirs[0].files, vec![ss("docs/guide.md")]);
    let src = &tree.dirs[1];
    assert_eq!(src.files, vec![ss("src/lib.rs"), ss("src/main.rs")], "files sort within a dir");
    assert_eq!(src.dirs.len(), 1);
    assert_eq!(src.dirs[0].path.as_str(), "src/ui");
    assert_eq!(src.dirs[0].files, vec![ss("src/ui/panel.rs")]);
}

#[test]
fn tree_skips_empty_segments() {
    let tree = build_file_tree(&[ss("a//double.rs"), ss("a/b/c.rs"), ss("/lead.rs")]);
    let a = tree.dirs.iter().find(|d| d.name.as_str() == "a").unwrap();
    assert_eq!(a.files, vec![ss("a//double.rs")], "the file keeps its original path");
    assert_eq!(a.dirs[0].files, vec![ss("a/b/c.rs")]);
    assert_eq!(tree.files, vec![ss("/lead.rs")]);
}

#[test]
fn empty_tree_has_no_children() {
    let tree = build_file_tree(&[]);
    assert_eq!(tree, DirNode::default());
}

#[test]
fn mention_appends_to_plain_draft() {
    assert_eq!(mention_text("", "src/lib.rs"), "@src/lib.rs ");
    assert_eq!(mention_text("check this", "src/lib.rs"), "check this @src/lib.rs ");
    assert_eq!(mention_text("check this ", "src/lib.rs"), "check this @src/lib.rs ");
}

#[test]
fn mention_replaces_an_open_query() {
    assert_eq!(mention_text("see @sr", "src/lib.rs"), "see @src/lib.rs ");
    assert_eq!(mention_text("@sr", "src/lib.rs"), "@src/lib.rs ");
    // `user@host` isn't a query — the @ isn't at a word boundary.
    assert_eq!(mention_text("mail user@host", "src/lib.rs"), "mail user@host @src/lib.rs ");
    // A completed mention followed by a word isn't a query either.
    assert_eq!(mention_text("@a.rs next", "src/lib.rs"), "@a.rs next @src/lib.rs ");
}

/// Switch to the Files tab with a canned file list loaded.
fn open_explorer(ws: &Entity<Workspace>, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    ws.update(cx, |this, cx| {
        this.project_files = canned_files();
        this.set_sidebar_tab(SidebarTab::Files, cx);
    });
    window.draw(cx).clear(cx);
}

#[test]
fn explorer_tab_renders_tree_and_expands_dirs() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("explorer").is_none(), "explorer starts on the Chats tab");

        open_explorer(&ws, window, cx);
        assert!(window.find("explorer").visible(), "Files tab mounts the explorer");
        // Top-level dirs seed expanded: their children render immediately.
        assert!(window.find(("explorer-dir", 0usize)).visible(), "docs dir row");
        assert!(window.find(("explorer-file", 1usize)).visible(), "docs/guide.md under expanded docs");
        assert!(window.find(("explorer-dir", 2usize)).visible(), "src dir row");
        assert!(window.find(("explorer-dir", 3usize)).visible(), "nested src/ui row");
        assert!(window.find(("explorer-file", 4usize)).visible(), "src/lib.rs under expanded src");
        // src/ui stays collapsed — panel.rs isn't in the tree yet.
        assert!(window.try_find(("explorer-file", 7usize)).is_none(), "src/ui children hidden while collapsed");

        // Expand src/ui → panel.rs appears; collapse src → its subtree goes.
        window.click(("explorer-dir", 3usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("explorer-file", 4usize)).visible(), "src/ui/panel.rs shows after expanding");
        window.click(("explorer-dir", 2usize), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find(("explorer-file", 4usize)).is_none(), "collapsing src hides its subtree");
        let expanded = &ws.read(cx).explorer.expanded;
        assert!(expanded.contains("docs") && expanded.contains("src/ui") && !expanded.contains("src"));
    });
}

#[test]
fn clicking_a_file_mentions_it_in_the_composer() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        // Seed a partial mention — the click replaces the open query.
        ws.update(cx, |this, cx| {
            this.composer.update(cx, |s, cx| s.set_value("see @sr", window, cx));
        });
        window.click(("explorer-file", 4usize), cx); // src/lib.rs
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).explorer.selected.as_deref(), Some("src/lib.rs"));
    });
    assert_eq!(composer_value(&ws, cx), "see @src/lib.rs ");
    cx.update(|window, cx| {
        // A second click appends — the draft ends in whitespace.
        window.click(("explorer-file", 6usize), cx); // README.md
        window.draw(cx).clear(cx);
    });
    assert_eq!(composer_value(&ws, cx), "see @src/lib.rs @README.md ");
}

#[test]
fn cmd_shift_e_toggles_the_explorer_tab() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        cx.bind_keys([KeyBinding::new("cmd-shift-e", crate::ToggleExplorer, Some("workspace"))]);
        window.draw(cx).clear(cx);

        window.press("cmd-shift-e", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).sidebar_tab, SidebarTab::Files, "cmd-shift-e opens Files");
        assert!(window.find("explorer").visible());

        window.press("cmd-shift-e", cx);
        window.draw(cx).clear(cx);
        assert_eq!(ws.read(cx).sidebar_tab, SidebarTab::Chats, "second press returns to Chats");
        assert!(window.try_find("explorer").is_none());
    });
}

#[test]
fn tab_buttons_switch_between_chats_and_files() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let chat_id = ws.read(cx).chats[0].id;
        assert!(window.find(("chat-row", chat_id)).visible(), "chat list on the Chats tab");

        window.click("tab-files", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("explorer").visible(), "tab button mounts the explorer");
        assert!(window.try_find(("chat-row", chat_id)).is_none(), "chat list unmounts");

        window.click("tab-chats", cx);
        window.draw(cx).clear(cx);
        assert!(window.find(("chat-row", chat_id)).visible(), "chat list returns");
    });
}
