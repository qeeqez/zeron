//! The shared file context menu — `file_menu` plus the Changes-only "Copy
//! Diff" item. Split from `open_in.rs` for the SLOC cap; re-exported there
//! so callers keep using `crate::open_in::file_menu` / `copy_diff_item`.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::workspace::Workspace;

use super::PreferredEditor;

/// One "Open in <editor>" item for the `Ask` submenu — a free fn so the
/// submenu fold stays under the nesting lint.
fn editor_pick_item(ws: &Entity<Workspace>, rel: &str, editor: PreferredEditor) -> PopupMenuItem {
    let ws = ws.clone();
    let rel = rel.to_string();
    PopupMenuItem::new(editor.label()).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| this.open_in_editor(&rel, Some(editor), cx));
    })
}

/// The "Copy Diff" item Changes rows append after `file_menu` — kept beside
/// it since both build the same row menu. Not part of `file_menu` itself:
/// explorer rows and the repo-root row have no diff to copy. `staged` is
/// the row's staged marker — a partially-staged file copies its `--cached`
/// half.
pub fn copy_diff_item(ws: &Entity<Workspace>, rel: &str, staged: bool) -> PopupMenuItem {
    let ws = ws.clone();
    let rel = rel.to_string();
    PopupMenuItem::new("Copy Diff").icon(IconName::FileDiff).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| this.copy_file_diff(&rel, staged, cx));
    })
}

/// "File History" + "Blame" — offered only for tracked files (the repo-root
/// row passes `rel` empty, untracked files fail `ls-files --error-unmatch`),
/// since both shell out to git and an untracked path has neither. The probe
/// is one `git ls-files` on menu open.
fn git_items(ws: &Entity<Workspace>, rel: &str, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
    if rel.is_empty() || !crate::git::tracked(ws.read(cx).project.root(), rel) {
        return menu;
    }
    let ws_log = ws.clone();
    let rel_log = rel.to_string();
    let ws_blame = ws.clone();
    let rel_blame = rel.to_string();
    menu.separator()
        .item(PopupMenuItem::new("File History").icon(IconName::GitCommitHorizontal).on_click(move |_, _w, cx| {
            ws_log.update(cx, |this, cx| this.open_file_history(&rel_log, cx));
        }))
        .item(PopupMenuItem::new("Blame").icon(IconName::UserSearch).on_click(move |_, _w, cx| {
            ws_blame.update(cx, |this, cx| this.open_file_blame(&rel_blame, cx));
        }))
}

/// The right-click menu shared by every file row: Changes entries, explorer
/// files, and — with `rel` empty — the repo root on the git branch row.
/// "Open in Editor" reads the preferred editor; `Ask` turns the item into a
/// submenu of concrete editors.
pub fn file_menu(ws: &Entity<Workspace>, rel: &str, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let preferred = ws.read(cx).preferred_editor;
    let ws_reveal = ws.clone();
    let rel_reveal = rel.to_string();
    let menu = menu.item(PopupMenuItem::new("Reveal in Finder").icon(IconName::FolderOpen).on_click(move |_, _w, cx| {
        ws_reveal.update(cx, |this, cx| this.reveal_in_finder(&rel_reveal, cx));
    }));
    let menu =
        if preferred == PreferredEditor::Ask {
            let ws_pick = ws.clone();
            let rel_pick = rel.to_string();
            menu.submenu("Open in Editor", window, cx, move |m, _w, _cx| {
                PreferredEditor::CHOICES
                    .into_iter()
                    .fold(m, |m, editor| m.item(editor_pick_item(&ws_pick, &rel_pick, editor)))
            })
        } else {
            let ws_open = ws.clone();
            let rel_open = rel.to_string();
            menu.item(PopupMenuItem::new(format!("Open in {}", preferred.label())).icon(IconName::ExternalLink).on_click(
                move |_, _w, cx| {
                    ws_open.update(cx, |this, cx| this.open_in_editor(&rel_open, None, cx));
                },
            ))
        };
    let ws_copy = ws.clone();
    let rel_copy = rel.to_string();
    let menu = menu.item(PopupMenuItem::new("Copy Path").icon(IconName::Copy).on_click(move |_, _w, cx| {
        ws_copy.update(cx, |this, cx| this.copy_file_path(&rel_copy, cx));
    }));
    git_items(ws, rel, menu, cx)
}

/// "New File…" / "New Folder…" items for a directory's menu — `dir` is the
/// project-relative parent the entry lands in ("" = the project root).
fn new_items(ws: &Entity<Workspace>, dir: &str, menu: PopupMenu) -> PopupMenu {
    let ws_file = ws.clone();
    let dir_file = dir.to_string();
    let ws_dir = ws.clone();
    let dir_dir = dir.to_string();
    menu.item(PopupMenuItem::new("New File…").icon(IconName::FilePlus).on_click(move |_, w, cx| {
        ws_file.update(cx, |this, cx| this.begin_new_file(&dir_file, w, cx));
    }))
    .item(PopupMenuItem::new("New Folder…").icon(IconName::FolderPlus).on_click(move |_, w, cx| {
        ws_dir.update(cx, |this, cx| this.begin_new_folder(&dir_dir, w, cx));
    }))
}

/// "Rename…" + "Delete" — the tail of every explorer row menu. `is_dir`
/// picks `remove_dir_all` over `remove_file` in `delete_path`, which also
/// runs the native confirm.
fn edit_items(ws: &Entity<Workspace>, rel: &str, is_dir: bool, menu: PopupMenu) -> PopupMenu {
    let ws_rename = ws.clone();
    let rel_rename = rel.to_string();
    let ws_delete = ws.clone();
    let rel_delete = rel.to_string();
    menu.separator()
        .item(PopupMenuItem::new("Rename…").icon(IconName::Pencil).on_click(move |_, w, cx| {
            ws_rename.update(cx, |this, cx| this.begin_rename_path(&rel_rename, w, cx));
        }))
        .item(PopupMenuItem::new("Delete").icon(IconName::Trash).on_click(move |_, w, cx| {
            ws_delete.update(cx, |this, cx| this.delete_path(&rel_delete, is_dir, w, cx));
        }))
}

/// A file row's menu: the shared `file_menu` items plus Rename/Delete.
pub fn explorer_file_menu(
    ws: &Entity<Workspace>, rel: &str, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    edit_items(ws, rel, false, file_menu(ws, rel, menu, window, cx))
}

/// A directory row's menu: the create items lead, then the shared
/// `file_menu` items, then Rename/Delete.
pub fn explorer_dir_menu(
    ws: &Entity<Workspace>, rel: &str, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu = file_menu(ws, rel, new_items(ws, rel, menu).separator(), window, cx);
    edit_items(ws, rel, true, menu)
}

/// The explorer header's right-click menu — create items at the project
/// root only (reveal/open/copy live on the git branch row's `file_menu`).
pub fn explorer_root_menu(ws: &Entity<Workspace>, menu: PopupMenu, _window: &mut Window, _cx: &mut Context<PopupMenu>) -> PopupMenu {
    new_items(ws, "", menu)
}
