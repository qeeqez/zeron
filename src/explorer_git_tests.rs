//! Explorer git decorations: the status letters on file rows and the dirty
//! dots on directory rows are a pure view over `Workspace::changes` — the
//! same snapshot the Changes panel renders. Unit tests pin the badge map;
//! headless runs mount the real sidebar and read the badges' a11y labels.
//! Narrow imports on purpose (see `composer_testutil`).

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext};

use crate::changes_ui_tests::change;
use crate::composer_testutil::open_workspace;
use crate::git::{ChangeStatus, FileChange};
use crate::views::explorer_git::{GitDecorations, Tone};
use crate::views::sidebar::SidebarTab;
use crate::workspace::Workspace;

fn ss(s: &str) -> gpui_kit::SharedString {
    s.into()
}

/// A tree that exercises every badge: staged add, untracked, modified,
/// deleted, conflicted, plus a clean file and a clean dir.
fn canned_files() -> Vec<gpui_kit::SharedString> {
    [
        "docs/guide.md", "docs/index.md", "src/added.rs", "src/lib.rs", "src/main.rs", "src/ui/panel.rs", "README.md",
    ]
    .iter()
    .map(|s| ss(s))
    .collect()
}

/// One change per status, aimed at the canned tree.
fn canned_changes() -> Vec<FileChange> {
    let mut staged_add = change("src/added.rs", ChangeStatus::Added, 4, 0);
    staged_add.staged = true;
    vec![
        change("docs/guide.md", ChangeStatus::Deleted, 0, 9),
        staged_add,
        change("src/lib.rs", ChangeStatus::Modified, 3, 1),
        change("src/ui/new.rs", ChangeStatus::Added, 7, 0), // untracked, not in the tree
        change("README.md", ChangeStatus::Conflicted, 0, 0),
    ]
}

/// Switch to the Files tab with the canned tree + changes loaded.
fn open_explorer(ws: &Entity<Workspace>, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    ws.update(cx, |this, cx| {
        this.project_files = canned_files();
        this.changes = canned_changes();
        this.set_sidebar_tab(SidebarTab::Files, cx);
    });
    window.draw(cx).clear(cx);
}

#[test]
fn decorations_map_files_and_aggregate_dirs() {
    let git = GitDecorations::build(&canned_changes());
    assert_eq!(git.file("src/lib.rs").map(|b| (b.letter, b.tone)), Some(("M", Tone::Warning)));
    assert_eq!(git.file("docs/guide.md").map(|b| (b.letter, b.tone)), Some(("D", Tone::Danger)));
    assert_eq!(git.file("src/added.rs").map(|b| (b.letter, b.tone)), Some(("A", Tone::Success)));
    assert_eq!(git.file("src/ui/new.rs").map(|b| (b.letter, b.tone)), Some(("?", Tone::Info)));
    assert_eq!(git.file("README.md").map(|b| (b.letter, b.tone)), Some(("C", Tone::Danger)));
    assert!(git.file("src/main.rs").is_none(), "clean file has no badge");
    // Ancestors of changed paths are dirty — including dirs whose only dirty
    // descendant isn't in the tree (untracked src/ui/new.rs).
    assert_eq!(git.dir("src"), Some(Tone::Warning), "src's worst descendant is modified");
    assert_eq!(git.dir("src/ui"), Some(Tone::Info));
    assert_eq!(git.dir("docs"), Some(Tone::Danger));
    assert!(git.dir("assets").is_none(), "clean dir stays clean");
}

#[test]
fn empty_changes_mean_no_decorations() {
    let git = GitDecorations::build(&[]);
    assert!(git.file("src/lib.rs").is_none());
    assert!(git.dir("src").is_none());
}

#[test]
fn file_rows_carry_status_letters() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        // Flatten order (dirs before files): docs(0) guide(1) index(2)
        // src(3) src/ui(4) added(5) lib(6) main(7) README(8); src/ui's
        // children stay collapsed.
        assert_eq!(window.find(("explorer-badge", 1usize)).label(), Some("git status: deleted"));
        assert_eq!(window.find(("explorer-badge", 5usize)).label(), Some("git status: added"));
        assert_eq!(window.find(("explorer-badge", 6usize)).label(), Some("git status: modified"));
        assert_eq!(window.find(("explorer-badge", 8usize)).label(), Some("git status: conflicted"));
        assert!(window.try_find(("explorer-badge", 2usize)).is_none(), "clean file has no badge");
        assert!(window.try_find(("explorer-badge", 7usize)).is_none(), "clean file has no badge");
        // The untracked src/ui/new.rs isn't in the tree — no row, no badge.
        assert!(window.try_find(("explorer-badge", 4usize)).is_none(), "dir rows carry no letter");
    });
}

#[test]
fn dirty_dirs_show_a_dot() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        open_explorer(&ws, window, cx);
        assert_eq!(window.find(("explorer-dirty", 0usize)).label(), Some("contains changes"), "docs is dirty");
        assert_eq!(window.find(("explorer-dirty", 3usize)).label(), Some("contains changes"), "src is dirty");
        assert_eq!(window.find(("explorer-dirty", 4usize)).label(), Some("contains changes"), "src/ui is dirty via untracked new.rs");
    });
}

#[test]
fn no_changes_render_no_badges() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project_files = canned_files();
            this.set_sidebar_tab(SidebarTab::Files, cx);
        });
        window.draw(cx).clear(cx);
        assert!(window.find("explorer").visible(), "explorer renders");
        assert!(window.try_find(("explorer-badge", 0usize)).is_none(), "no file badges without changes");
        assert!(window.try_find(("explorer-dirty", 0usize)).is_none(), "no dir dots without changes");
        assert!(window.find(("explorer-file", 5usize)).visible(), "rows still render");
    });
}
