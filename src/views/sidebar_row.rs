//! Chat rows in the sidebar. Every row — chat entries, "New chat", and the
//! settings nav items — renders through the shared `NavRow` component so
//! padding, colors, hover and selected states stay identical across screens.
//! `chat_row` adds what stock rows can't express: a hover-revealed "…" menu
//! button and an inline rename editor that replaces the title label.

mod drag_ghost;
mod sidebar_menu;

use drag_ghost::ChatDragGhost;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Input, InputState};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{Chat, MessageKind};
use crate::views::nav_row::NavRow;
use crate::workspace::Workspace;

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
        .suffix(row_suffix(cx.entity(), chat_id, flags, RowStatus::of(chat), ws.send_queue.len(chat_id)))
    }
}

/// Row click → select the chat (resolved by id — positions shift on
/// delete). A plain click also drops the multi-selection — including on
/// the active row, where `select_chat` early-returns. Focus lands on the
/// composer either way: the sidebar wrap is focusable now, so without the
/// explicit refocus a click on the already-active row would strand the
/// keyboard on the sidebar and typing would go nowhere.
fn select_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.clear_chat_selection(cx);
        if let Some(ix) = this.chat_index(chat_id) {
            this.select_chat(ix, window, cx);
        }
        this.composer.update(cx, |s, cx| s.focus(window, cx));
    });
}

/// Cmd-click on a row → toggle the chat in the bulk-op selection and hand
/// the keyboard to the sidebar, so Enter renames the selected row (see
/// `Workspace::rename_selected_row`).
fn toggle_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.toggle_chat_selection(chat_id, cx);
        let sidebar = this.sidebar_focus.clone();
        window.focus(&sidebar, cx);
    });
}

/// Double-click on the title → open the inline rename editor.
fn rename_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        if let Some(ix) = this.chat_index(chat_id) {
            this.start_inline_rename(ix, window, cx);
        }
    });
}

/// Inline title editor — Enter commits, Escape cancels, and a mouse-down
/// anywhere outside the field commits (Finder-style).
fn rename_editor(ws: Entity<Workspace>, input: Entity<InputState>, chat_id: u64) -> impl Fn(&mut Window, &mut App) -> AnyElement {
    let ws_out = ws.clone();
    let ws_enter = ws.clone();
    let ws_esc = ws.clone();
    move |_, _| {
        div()
            .flex_1()
            .min_w_0()
            .on_mouse_down_out({
                let ws = ws_out.clone();
                move |_, window, cx| {
                    ws.update(cx, |this, cx| this.commit_rename(window, cx));
                }
            })
            .on_action({
                let ws = ws_enter.clone();
                move |_: &InputEnter, window, cx| {
                    cx.stop_propagation();
                    ws.update(cx, |this, cx| this.commit_rename(window, cx));
                }
            })
            .on_action({
                let ws = ws_esc.clone();
                move |_: &InputEscape, window, cx| {
                    cx.stop_propagation();
                    ws.update(cx, |this, cx| this.cancel_inline_rename(window, cx));
                }
            })
            .child(Input::new(&input).id(("rename-input", chat_id)).xsmall().w_full())
            .into_any_element()
    }
}

/// What the row's status slot should announce, most actionable first.
struct RowStatus {
    running: bool,
    unread: bool,
    needs_approval: bool,
}

impl RowStatus {
    /// A live `respond` channel means the turn is parked on the user —
    /// impossible on a pending_load chat: approvals only reach hydrated
    /// transcripts and the channel never survives a reload.
    fn of(chat: &Chat) -> Self {
        let pending = |m: &crate::model::ChatMessage| matches!(&m.kind, MessageKind::Approval(a) if a.respond.is_some());
        Self {
            running: chat.running,
            unread: chat.unread,
            needs_approval: chat.messages.iter().any(pending),
        }
    }
}

/// Trailing row content: queued-count chip, an approval-waiting shield /
/// spinner while a reply streams / unread dot, then the "…" button that
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
            } else if status.running {
                IconName::LoaderCircle.into_any_element()
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

/// The "+N" queued-send chip: same pill geometry as the titlebar's unread
/// badge, muted instead of red. Clicking selects the chat — its composer
/// holds the queue UI — and stops the row's own click (rename on
/// double-click) from seeing the press.
fn queue_badge(chat_id: u64, queued: usize, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let tip = format!("{queued} queued");
    div()
        .id(("queue-badge", chat_id))
        .test_support()
        .aria_label(tip.clone())
        .min_w(px(14.))
        .h(px(14.))
        .px(px(3.))
        .rounded_full()
        .bg(cx.theme().muted)
        .text_color(cx.theme().muted_foreground)
        .text_size(px(9.))
        .flex()
        .items_center()
        .justify_center()
        .child(format!("+{queued}"))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .on_click({
            let ws = ws.clone();
            move |_, window, cx| {
                cx.stop_propagation();
                select_row(&ws, chat_id, window, cx);
            }
        })
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "../sidebar_dnd_tests.rs"]
mod sidebar_dnd_tests;
