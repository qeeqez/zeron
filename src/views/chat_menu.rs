//! The chat header's ⋯ menu and worktree badge — split from `chat_view.rs`
//! for the SLOC cap.

mod badges;
mod bookmarks;
mod color;
mod compare;
mod continue_with;
mod schedule;
mod worktree;
pub(crate) use badges::{color_dot, context_chip, temp_badge, worktree_badge};
use bookmarks::bookmarks_submenu;
use color::chat_color_submenu;
pub(crate) use color::color_submenu;
use compare::compare_item;
use continue_with::continue_with_submenu;
use schedule::schedule_item;
use worktree::worktree_items;

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
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
    /// The chat has 2+ messages and no reply running — gates "Split chat…".
    pub can_split: bool,
    /// A backend turn is running — gates "Merge into project" (the
    /// worktree's files are still moving).
    pub running: bool,
    /// No messages yet — disables "Copy transcript" (nothing to copy).
    pub empty: bool,
}

/// Pin/rename/export/copy/snapshots/word-wrap — the ⋯ menu on the chat
/// titlebar. Worktree chats also get reveal/open items for their checkout.
pub fn chat_menu(
    menu: PopupMenu, ws: &Entity<Workspace>, state: ChatMenuState, window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let ChatMenuState {
        pinned,
        word_wrap,
        color,
        worktree,
        ephemeral,
        can_split,
        running,
        empty,
    } = state;
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_export = ws.clone();
    let ws_export_html = ws.clone();
    let ws_copy = ws.clone();
    let ws_wrap = ws.clone();
    let ws_snap = ws.clone();
    let ws_fork = ws.clone();
    let ws_window = ws.clone();
    let ws_temp = ws.clone();
    let ws_info = ws.clone();
    let ws_budget = ws.clone();
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
            move |m, _w, _cx| chat_color_submenu(&ws, color, m)
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
        .item(PopupMenuItem::new("Budget alert…").icon(IconName::CircleDollarSign).on_click(move |_, window, cx| {
            ws_budget.update(cx, |this, cx| {
                let id = this.chats[this.active].id;
                this.open_chat_budget(id, window, cx);
            });
        }))
        .item(schedule_item(ws, ephemeral))
        .item(PopupMenuItem::new("Export").icon(IconName::Share).disabled(ephemeral).on_click(move |_, _, cx| {
            ws_export.update(cx, |this, cx| this.export_active(cx));
        }))
        .item(
            PopupMenuItem::new("Export HTML…")
                .icon(IconName::FileCode)
                .disabled(ephemeral)
                .on_click(move |_, _, cx| {
                    ws_export_html.update(cx, |this, cx| this.export_active_html(cx));
                }),
        )
        .item(
            PopupMenuItem::new("Copy transcript")
                .icon(IconName::Copy)
                .disabled(empty)
                .on_click(move |_, window, cx| {
                    ws_copy.update(cx, |this, cx| this.copy_transcript(window, cx));
                }),
        )
        .item(PopupMenuItem::new("Fork chat").icon(IconName::GitFork).on_click(move |_, window, cx| {
            ws_fork.update(cx, |this, cx| {
                let ix = this.active;
                this.fork_chat(ix, None, window, cx);
            });
        }))
        .item(PopupMenuItem::new("Split chat…").icon(IconName::Scissors).disabled(!can_split).on_click({
            let ws = ws.clone();
            move |_, window, cx| {
                ws.update(cx, |this, cx| {
                    let id = this.chats[this.active].id;
                    this.open_split_dialog(id, window, cx);
                });
            }
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
    // "Continue with" — fork the transcript onto another provider — and
    // "Compare providers…" — fork it onto several at once. After the
    // chat-shape items, before the conditional tail.
    let menu = continue_with_submenu(menu, ws, window, cx);
    let menu = compare_item(menu, ws, cx);
    // "Copy resume command" only exists when the chat is bound to a backend
    // thread AND the backend has a CLI resume (codex/claude).
    let menu = if ws.read(cx).resume_command().is_some() {
        let ws_resume = ws.clone();
        menu.item(PopupMenuItem::new("Copy resume command").icon(IconName::Terminal).on_click(move |_, _, cx| {
            ws_resume.update(cx, |this, cx| this.copy_resume_command(cx));
        }))
    } else {
        menu
    };
    let menu = bookmarks_submenu(menu, ws, window, cx);
    let menu = if worktree { worktree_items(menu, ws, running, window, cx) } else { menu };
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
    .item(PopupMenuItem::new("Chat info").icon(IconName::Info).on_click(move |_, _window, cx| {
        ws_info.update(cx, |this, cx| this.open_chat_info(cx));
    }))
}
