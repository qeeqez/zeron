//! UI tests for the file menu's "Blame" and "File History" — the menu items
//! on Changes and explorer rows and the overlay they open. Declared as
//! `crate::git::blame::blame_ui_tests` via `#[path]` so `main.rs` stays
//! under the SLOC cap; helpers live in `blame_tests.rs`.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

use super::blame_tests::{click_menu_item, commit_as, menu_labels, mounted, run, temp_repo, until};
use crate::git::ChangeStatus;
use crate::views::file_inspect::FileInspect;

/// Changes and explorer rows share `file_menu` — both offer the git items
/// for a tracked file.
#[test]
fn menu_offers_history_and_blame_on_tracked_files() {
    let Some(dir) = temp_repo("menu") else { return };
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "alice", "init"));

    let mut app = TestAppContext::single();
    let (ws, cx) = mounted(&mut app, &dir, "f.txt");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.project_files = vec!["f.txt".into()];
            this.set_sidebar_tab(crate::views::sidebar::SidebarTab::Files, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "File History"), "changes row offers history: {labels:?}");
        assert!(labels.iter().any(|l| l == "Blame"), "changes row offers blame: {labels:?}");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("explorer-file", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "File History"), "explorer row offers history: {labels:?}");
        assert!(labels.iter().any(|l| l == "Blame"), "explorer row offers blame: {labels:?}");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Untracked files have no blame or history — the items stay hidden.
#[test]
fn menu_hides_git_items_on_untracked_files() {
    let Some(dir) = temp_repo("menu-untracked") else { return };
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "alice", "init"));
    std::fs::write(dir.join("new.txt"), "x\n").unwrap();

    let mut app = TestAppContext::single();
    let (ws, cx) = mounted(&mut app, &dir, "new.txt");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.changes = vec![crate::changes_ui_tests::change("new.txt", ChangeStatus::Added, 1, 0)];
            cx.notify();
        });
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let labels = menu_labels(window);
        assert!(labels.iter().any(|l| l == "Reveal in Finder"), "menu opened: {labels:?}");
        assert!(!labels.iter().any(|l| l == "File History"), "untracked file has no history: {labels:?}");
        assert!(!labels.iter().any(|l| l == "Blame"), "untracked file has no blame: {labels:?}");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// "File History" opens the overlay and lands one row per commit; clicking a
/// row expands that commit's diff for the file.
#[test]
fn history_overlay_lists_commits_and_expands_diffs() {
    let Some(dir) = temp_repo("history-ui") else { return };
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "alice", "init"));
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "bob", "extend"));

    let mut app = TestAppContext::single();
    let (ws, cx) = mounted(&mut app, &dir, "f.txt");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "File History", cx);
    });
    until(&ws, cx, |w| matches!(&w.file_inspect, Some(FileInspect::History { result: Some(Ok(_)), .. })));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("file-inspect-overlay").visible(), "overlay renders");
        assert!(window.find(("file-history-row", 0usize)).visible(), "newest commit first");
        assert!(window.find(("file-history-row", 1usize)).visible(), "older commit listed");
        window.click(("file-history-row", 0usize), cx);
    });
    until(&ws, cx, |w| matches!(&w.file_inspect, Some(FileInspect::History { result: Some(Ok(c)), .. }) if c[0].diff.is_some()));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("file-history-diff", 0usize)).visible(), "row expands the file's diff");
        window.click("file-inspect-close", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("file-inspect-overlay").is_none(), "close dismisses the overlay");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Blame" opens the overlay with one monospace row per line; clicking a row
/// copies the commit hash.
#[test]
fn blame_overlay_lists_lines_and_copies_sha() {
    let Some(dir) = temp_repo("blame-ui") else { return };
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").unwrap();
    assert!(run(&dir, &["add", "f.txt"]));
    assert!(commit_as(&dir, "alice", "init"));

    let mut app = TestAppContext::single();
    let (ws, cx) = mounted(&mut app, &dir, "f.txt");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.right_click(("change-row", 0usize), cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        click_menu_item(window, "Blame", cx);
    });
    until(&ws, cx, |w| matches!(&w.file_inspect, Some(FileInspect::Blame { result: Some(Ok(_)), .. })));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("file-inspect-overlay").visible(), "overlay renders");
        let row = window.find(("file-blame-row", 0usize));
        assert!(row.visible(), "blame row renders");
        assert!(row.label().unwrap_or_default().contains("one"), "row carries the line text");
        window.click(("file-blame-row", 0usize), cx);
    });
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default());
    let sha = ws.read_with(cx, |w, _| match &w.file_inspect {
        Some(FileInspect::Blame { result: Some(Ok(lines)), .. }) => lines[0].sha.clone(),
        _ => String::new(),
    });
    assert_eq!(clip, sha, "clicking a blame row copies its sha");
    let _ = std::fs::remove_dir_all(&dir);
}
