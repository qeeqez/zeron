//! The chat header's ⋯ menu and worktree badge — split from `chat_view.rs`
//! for the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Per-render state the ⋯ menu needs — bundled to stay under the arg-count
/// lint. `worktree` gates the worktree-only items.
#[derive(Clone, Copy)]
pub struct ChatMenuState {
    pub pinned: bool,
    pub word_wrap: bool,
    /// The chat's color tag (`Chat.color`) — the Color submenu checks it.
    pub color: Option<crate::model::ChatColor>,
    /// The chat runs in a per-thread git worktree (`Chat.worktree`).
    pub worktree: bool,
    /// Temporary chat (`Chat.ephemeral`) — disables the items that need a
    /// persisted chat (export, open-in-new-window).
    pub ephemeral: bool,
}

/// The color-tag dot — one shape for the sidebar row, the titlebar and the
/// ⋯ menu's swatches. `id` keeps it findable in headless tests.
pub fn color_dot(id: impl Into<ElementId>, color: crate::model::ChatColor, size: Pixels) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .w(size)
        .h(size)
        .rounded_full()
        .flex_shrink_0()
        .bg(color.hsla())
        .into_any_element()
}

/// The worktree chip on the chat titlebar — a muted icon + "worktree" label
/// whose tooltip carries the checkout path. `id` keeps it findable in
/// headless tests.
pub fn worktree_badge(id: &'static str, workdir: &str, cx: &App) -> AnyElement {
    let tip = workdir.to_string();
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::FolderGit)
        .child("worktree")
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .into_any_element()
}

/// The "Temporary" chip on the chat titlebar — same muted styling as the
/// worktree badge; a chat can carry both. `id` keeps it findable in
/// headless tests.
pub fn temp_badge(id: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::Ghost)
        .child("Temporary")
        .tooltip(|window, cx| Tooltip::new("Not saved — closes with the chat").build(window, cx))
        .into_any_element()
}

/// Pin/rename/export/copy/snapshots/word-wrap — the ⋯ menu on the chat
/// titlebar. Worktree chats also get reveal/open items for their checkout.
pub fn chat_menu(
    menu: PopupMenu, ws: &Entity<Workspace>, state: ChatMenuState, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let ChatMenuState { pinned, word_wrap, color, worktree, ephemeral } = state;
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_export = ws.clone();
    let ws_copy = ws.clone();
    let ws_wrap = ws.clone();
    let ws_snap = ws.clone();
    let ws_fork = ws.clone();
    let ws_window = ws.clone();
    let ws_temp = ws.clone();
    let menu = menu
        .item(PopupMenuItem::new("New Temporary Chat").icon(IconName::Ghost).on_click(move |_, _, cx| {
            ws_temp.update(cx, |this, cx| this.new_temp_chat(cx));
        }))
        .item(
            PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                .icon(IconName::Star)
                .on_click(move |_, _, cx| {
                    ws_pin.update(cx, |this, cx| this.toggle_pin(this.active, cx));
                }),
        )
        .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, window, cx| {
            ws_rename.update(cx, |this, cx| this.rename_active(window, cx));
        }))
        .submenu("Color", window, cx, {
            let ws = ws.clone();
            move |m, _w, _cx| color_submenu(&ws, color, m)
        })
        .item(PopupMenuItem::new("Custom instructions…").icon(IconName::NotebookPen).on_click({
            let ws = ws.clone();
            move |_, window, cx| {
                ws.update(cx, |this, cx| {
                    let id = this.chats[this.active].id;
                    this.open_chat_instructions(id, window, cx);
                });
            }
        }))
        .item(PopupMenuItem::new("Export").icon(IconName::Share).disabled(ephemeral).on_click(move |_, _, cx| {
            ws_export.update(cx, |this, cx| this.export_active(cx));
        }))
        .item(PopupMenuItem::new("Copy transcript").icon(IconName::Copy).on_click(move |_, _, cx| {
            ws_copy.update(cx, |this, cx| this.copy_transcript(cx));
        }))
        .item(PopupMenuItem::new("Fork chat").icon(IconName::GitFork).on_click(move |_, window, cx| {
            ws_fork.update(cx, |this, cx| {
                let ix = this.active;
                this.fork_chat(ix, None, window, cx);
            });
        }))
        .item(
            PopupMenuItem::new("Open in New Window")
                .icon(IconName::WindowRestore)
                .disabled(ephemeral)
                .on_click(move |_, _, cx| {
                    ws_window.update(cx, |this, cx| {
                        let id = this.chats[this.active].id;
                        this.open_chat_in_new_window(id, cx);
                    });
                }),
        );
    let menu = bookmarks_submenu(menu, ws, window, cx);
    let menu = if worktree { worktree_items(menu, ws, window, cx) } else { menu };
    menu.item(PopupMenuItem::new("Snapshots").icon(IconName::Camera).on_click(move |_, _, cx| {
        ws_snap.update(cx, |this, cx| this.toggle_snapshots_panel(cx));
    }))
    .item(PopupMenuItem::new("Word wrap").icon(IconName::Check).checked(word_wrap).on_click(move |_, _, cx| {
        ws_wrap.update(cx, |this, cx| {
            this.word_wrap = !this.word_wrap;
            this.save_settings();
            // Every message's height changed — remeasure the whole list.
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
            cx.notify();
        });
    }))
}

/// The "Bookmarks" submenu: one row per starred message, numbered, labeled
/// with the first ~60 chars of its text. Clicking scrolls the transcript to
/// the message via `scroll_to_message`; an empty list shows a disabled
/// "No bookmarks" row.
fn bookmarks_submenu(menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let this = ws.read(cx);
    let bookmarks: Vec<(usize, String)> = this.chats[this.active]
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.bookmarked)
        .map(|(ix, m)| (ix, bookmark_label(m)))
        .collect();
    let ws = ws.clone();
    menu.submenu_with_icon(Some(IconName::Star.into()), "Bookmarks", window, cx, move |m, _w, _cx| {
        if bookmarks.is_empty() {
            return m.item(PopupMenuItem::new("No bookmarks").disabled(true));
        }
        bookmarks.iter().enumerate().fold(m, |m, (n, (ix, label))| {
            let ws = ws.clone();
            let ix = *ix;
            let label = format!("{}. {label}", n + 1);
            m.item(PopupMenuItem::new(label).on_click(move |_, _w, cx| {
                ws.update(cx, |this, cx| this.scroll_to_message(ix, cx));
            }))
        })
    })
}

/// One-line preview for a bookmarked message — whitespace squashed, clipped
/// at 60 chars so long replies stay one menu row.
fn bookmark_label(msg: &crate::model::ChatMessage) -> String {
    let squashed = msg.markdown().split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped: String = squashed.chars().take(60).collect();
    if squashed.chars().count() > 60 { format!("{clipped}…") } else { clipped }
}

/// The worktree-only section of the ⋯ menu: reveal the checkout in Finder
/// and open it in the preferred editor (`Ask` expands to a picker, same as
/// the file menu). Paths resolve at click time so a deleted worktree falls
/// back to the project root via `workdir_for`.
fn worktree_items(menu: PopupMenu, ws: &Entity<Workspace>, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let preferred = ws.read(cx).preferred_editor;
    let ws_reveal = ws.clone();
    let menu = menu.item(
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

/// The "Color" submenu: one swatch row per `ChatColor` plus "None" to clear.
/// The current tag reads checked — the swatch carries `aria_toggled` so
/// tests see the same state the check icon shows.
fn color_submenu(ws: &Entity<Workspace>, current: Option<crate::model::ChatColor>, menu: PopupMenu) -> PopupMenu {
    use gpui_kit::accesskit::Toggled;
    let menu = crate::model::ChatColor::ALL.into_iter().fold(menu, |m, color| {
        let checked = current == Some(color);
        let ws = ws.clone();
        m.item(
            PopupMenuItem::element(move |_, _| {
                div()
                    .id(format!("color-swatch-{}", color.name()))
                    .test_support()
                    .role(Role::MenuItemRadio)
                    .aria_toggled(if checked { Toggled::True } else { Toggled::False })
                    .aria_label(color.label())
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(color_dot(format!("swatch-dot-{}", color.name()), color, px(10.)))
                    .child(color.label())
            })
            .checked(checked)
            .on_click(move |_, _w, cx| {
                ws.update(cx, |this, cx| {
                    let id = this.chats[this.active].id;
                    this.set_chat_color(id, Some(color), cx);
                });
            }),
        )
    });
    let ws = ws.clone();
    menu.item(PopupMenuItem::new("None").checked(current.is_none()).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| {
            let id = this.chats[this.active].id;
            this.set_chat_color(id, None, cx);
        });
    }))
}
