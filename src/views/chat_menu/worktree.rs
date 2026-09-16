//! The ⋯ menu's worktree-only section — split from `chat_menu.rs` for the
//! SLOC cap (same pattern as `bookmarks.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::workspace::Workspace;

/// The worktree-only section of the ⋯ menu: merge the checkout's delta
/// back into the project, reveal it in Finder and open it in the
/// preferred editor (`Ask` expands to a picker, same as the file menu).
/// Paths resolve at click time so a deleted worktree falls back to the
/// project root via `workdir_for`. Merge is disabled while the chat's
/// turn runs — its files are still moving.
pub(super) fn worktree_items(
    menu: PopupMenu, ws: &Entity<Workspace>, running: bool, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let preferred = ws.read(cx).preferred_editor;
    let ws_merge = ws.clone();
    let ws_reveal = ws.clone();
    let menu = menu
        .item(
            PopupMenuItem::new("Merge into project")
                .icon(IconName::GitMerge)
                .disabled(running)
                .on_click(move |_, _w, cx| {
                    ws_merge.update(cx, |this, cx| this.merge_worktree_into_project(cx));
                }),
        )
        .item(
            PopupMenuItem::new("Reveal Worktree in Finder")
                .icon(IconName::FolderOpen)
                .on_click(move |_, _w, cx| {
                    ws_reveal.update(cx, |this, cx| {
                        let dir = worktree_dir(this);
                        this.reveal_path_in_finder(&dir, cx);
                    });
                }),
        );
    if preferred == crate::open_in::PreferredEditor::Ask {
        let ws_pick = ws.clone();
        menu.submenu("Open Worktree in Editor", window, cx, move |m, _w, _cx| {
            crate::open_in::PreferredEditor::CHOICES
                .into_iter()
                .fold(m, |m, editor| m.item(worktree_pick_item(&ws_pick, editor)))
        })
    } else {
        let ws_open = ws.clone();
        menu.item(
            PopupMenuItem::new(format!("Open Worktree in {}", preferred.label()))
                .icon(IconName::ExternalLink)
                .on_click(move |_, _w, cx| {
                    ws_open.update(cx, |this, cx| {
                        let dir = worktree_dir(this);
                        this.open_path_in_editor(&dir, None, cx);
                    });
                }),
        )
    }
}

/// One "Open in <editor>" pick for the `Ask` submenu — a free fn so the
/// submenu fold stays under the nesting lint.
fn worktree_pick_item(ws: &Entity<Workspace>, editor: crate::open_in::PreferredEditor) -> PopupMenuItem {
    let ws = ws.clone();
    PopupMenuItem::new(editor.label()).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| {
            let dir = worktree_dir(this);
            this.open_path_in_editor(&dir, Some(editor), cx);
        });
    })
}

/// The active chat's worktree directory — `workdir_for` falls back to the
/// project root when the checkout is gone, so the items never target a
/// missing path.
fn worktree_dir(this: &Workspace) -> std::path::PathBuf {
    crate::worktree::workdir_for(&this.chats[this.active], this.project.root())
}
