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
