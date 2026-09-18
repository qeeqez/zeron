//! Chat rows in the sidebar. Every row — chat entries, "New chat", and the
//! settings nav items — renders through the shared `NavRow` component so
//! padding, colors, hover and selected states stay identical across screens.
//! `chat_row` adds what stock rows can't express: a hover-revealed "…" menu
//! button and an inline rename editor that replaces the title label.

mod drag_ghost;
mod row_actions;
mod sidebar_menu;

use drag_ghost::ChatDragGhost;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{Chat, MessageKind};
use crate::views::nav_row::NavRow;
use crate::workspace::Workspace;

use row_actions::{queue_badge, rename_editor, rename_row, select_row, toggle_row};
use sidebar_menu::{RowFlags, RowMenu, chat_row_menu};

/// Drag payload a chat row carries — the id is what folder headers file on
/// drop; the title feeds the ghost preview. `Clone` because `NavRow`'s erased
/// drag source is `Fn`, not `FnOnce`.
#[derive(Clone)]
pub(super) struct ChatDrag {
    pub(super) id: u64,
    title: SharedString,
}

/// One chat row: icon + title (or inline rename editor) + status + "…" menu.
pub(super) fn chat_row(chat: &Chat, ix: usize, ws: &Workspace, cx: &mut Context<Workspace>) -> NavRow {
    let chat_id = chat.id;
    let flags = RowFlags {
        pinned: chat.pinned,
        archived: chat.archived,
        // The last chat can't be deleted — but a temporary last chat can
        // (it's replaced by a fresh normal chat), so keep Delete live.
        only_chat: ws.chats.len() <= 1 && !chat.ephemeral,
        worktree: chat.worktree,
        ephemeral: chat.ephemeral,
        on_screen: ix == ws.active || ws.secondary == Some(ix),
    };
    // Only an inline rename mounts the editor — a dialog rename shares
    // `ws.rename`, and its outside-click would commit behind the dialog.
    let renaming = ws.renaming == Some(chat_id) && ws.rename_mode == crate::workspace::RenameMode::Inline;
    let ws_click = cx.entity();
    let ws_drop = cx.entity();
    let row = NavRow::new(("chat-row", chat_id), chat.title.clone())
        .icon(if flags.pinned { IconName::StarFill } else { IconName::FileText })
        .active(ix == ws.active)
        .selected(ws.selected_chats.contains(&chat_id))
        .group(format!("chat-row-{chat_id}"))
        .context_menu({
            let ws = ws_click.clone();
            move |menu, window, cx| chat_row_menu(&ws, RowMenu { id: chat_id, flags }, menu, window, cx)
        });
    let row = match chat.color {
        Some(color) => row.leading(move |_, _| crate::views::chat_menu::color_dot(("chat-color-dot", chat_id), color, px(6.))),
        None => row,
    };
    // A colored folder marks its member rows with a thin left-edge bar —
    // the grouping cue that pairs with the header's color dot.
    let row = match ws.folder_colors.get(&chat.folder) {
        Some(color) => row.edge_accent(color.hsla()),
        None => row,
    };
    if renaming {
        row.body(rename_editor(ws_click.clone(), ws.rename.clone(), chat_id))
    } else {
        row.on_click(move |ev, window, cx| {
            if ev.modifiers().platform {
                toggle_row(&ws_click, chat_id, window, cx);
            } else if ev.click_count() >= 2 {
                rename_row(&ws_click, chat_id, window, cx);
            } else {
                select_row(&ws_click, chat_id, window, cx);
            }
        })
        .on_drag(ChatDrag { id: chat_id, title: chat.title.clone() }, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| ChatDragGhost { title: drag.title.clone() })
        })
        // Reorder/file target: the move handler runs for every row during a
        // `ChatDrag`, so it self-selects on `bounds.contains` and tracks the
        // hovered half as the drop edge.
        .drop_target(
            {
                let ws = ws_drop.clone();
                move |ev: &DragMoveEvent<ChatDrag>, _, cx| {
                    let hovered = ev.bounds.contains(&ev.event.position);
                    let above = ev.event.position.y < ev.bounds.origin.y + ev.bounds.size.height / 2.;
                    ws.update(cx, |this, cx| this.set_chat_drop(ev.drag(cx).id, chat_id, hovered.then_some(above), cx));
                }
            },
            {
                let ws = ws_drop.clone();
                move |drag: &ChatDrag, _, cx| ws.update(cx, |this, cx| this.drop_chat_on_row(drag.id, chat_id, cx))
            },
        )
        .drop_line(ws.chat_drop.and_then(|d| (d.row == chat_id).then_some(d.above)))
        .suffix(row_suffix(cx.entity(), chat_id, flags, RowStatus::of(chat, ws.chat_working(chat)), ws.send_queue.len(chat_id)))
    }
}

/// What the row's status slot should announce, most actionable first.
struct RowStatus {
    working: bool,
    unread: bool,
    needs_approval: bool,
}

impl RowStatus {
    /// A live `respond` channel means the turn is parked on the user —
    /// impossible on a pending_load chat: approvals only reach hydrated
    /// transcripts and the channel never survives a reload. `working` is
    /// the workspace's `chat_working` aggregate — the chat's own reply OR
    /// an attributed agent still running — so the spinner doesn't drop
    /// while subagents outlive their turn.
    fn of(chat: &Chat, working: bool) -> Self {
        let pending = |m: &crate::model::ChatMessage| matches!(&m.kind, MessageKind::Approval(a) if a.respond.is_some());
        Self {
            working,
            unread: chat.unread,
            needs_approval: chat.messages.iter().any(pending),
        }
    }
}

/// Trailing row content: queued-count chip, an approval-waiting shield /
/// spinner while the chat counts as working / unread dot, then the "…" button that
/// opens the same menu as right-click. The button stays visible while its
/// menu is up, even after the pointer leaves the row.
fn row_suffix(
    ws: Entity<Workspace>, chat_id: u64, flags: RowFlags, status: RowStatus, queued: usize,
) -> impl Fn(&mut Window, &mut App) -> AnyElement {
    move |window, cx| {
        let menu_open = window.use_keyed_state(("chat-menu-open", chat_id), cx, |_, _| false);
        h_flex()
            .items_center()
            .gap_1()
            // Worktree threads carry a small glyph ahead of the status
            // affordances — the header badge is the primary indicator.
            .when(flags.worktree, |d| {
                d.child(
                    div()
                        .id(("worktree-glyph", chat_id))
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::FolderGit),
                )
            })
            // Temporary chats carry a ghost glyph — the titlebar's
            // "Temporary" chip is the primary indicator.
            .when(flags.ephemeral, |d| {
                d.child(
                    div()
                        .id(("temp-glyph", chat_id))
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::Ghost),
                )
            })
            // Queued sends get a muted "+N" chip ahead of the status
            // affordances; clicking it opens the chat (the queue lives in
            // its composer, which select_chat focuses).
            .when(queued > 0, |d| d.child(queue_badge(chat_id, queued, &ws, cx)))
            // Approval outranks the spinner: the turn isn't streaming, it's
            // parked on a click — the persistent "needs you" marker after
            // the toast is gone.
            .child(if status.needs_approval {
                div()
                    .id(("approval-needed", chat_id))
                    .test_support()
                    .text_color(cx.theme().warning)
                    .child(IconName::ShieldAlert)
                    .into_any_element()
            } else if status.working {
                div()
                    .id(("chat-working", chat_id))
                    .test_support()
                    .child(IconName::LoaderCircle)
                    .into_any_element()
            } else if status.unread {
                div().w_2().h_2().rounded_full().bg(hsla(0.0, 0.0, 0.55, 1.0)).into_any_element()
            } else {
                div().into_any_element()
            })
            .child(
                div()
                    .id(("chat-menu", chat_id))
                    .test_support()
                    .when(!menu_open.read(cx), |this| this.invisible().group_hover(format!("chat-row-{chat_id}"), |style| style.visible()))
                    .child(
                        Button::new(("chat-menu-btn", chat_id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .dropdown_menu_with_anchor(Anchor::TopRight, {
                                let ws = ws.clone();
                                move |menu, window, cx| chat_row_menu(&ws, RowMenu { id: chat_id, flags }, menu, window, cx)
                            })
                            .on_open_change({
                                let menu_open = menu_open.clone();
                                move |open, _window, cx| {
                                    menu_open.update(cx, |state, _| *state = *open);
                                }
                            }),
                    ),
            )
            .into_any_element()
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "../sidebar_dnd_tests.rs"]
mod sidebar_dnd_tests;
