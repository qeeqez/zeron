//! Chat rows in the sidebar. Every row — chat entries, "New chat", and the
//! settings nav items — renders through the shared `NavRow` component so
//! padding, colors, hover and selected states stay identical across screens.
//! `chat_row` adds what stock rows can't express: a hover-revealed "…" menu
//! button and an inline rename editor that replaces the title label.

mod sidebar_menu;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Input, InputState};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::views::nav_row::NavRow;
use crate::workspace::Workspace;

use sidebar_menu::{RowFlags, RowMenu, chat_row_menu};

/// One chat row: icon + title (or inline rename editor) + status + "…" menu.
pub(super) fn chat_row(chat: &Chat, ix: usize, ws: &Workspace, cx: &mut Context<Workspace>) -> NavRow {
    let chat_id = chat.id;
    let flags = RowFlags {
        pinned: chat.pinned,
        archived: chat.archived,
        only_chat: ws.chats.len() <= 1,
        worktree: chat.worktree,
    };
    // Only an inline rename mounts the editor — a dialog rename shares
    // `ws.rename`, and its outside-click would commit behind the dialog.
    let renaming = ws.renaming == Some(chat_id) && ws.rename_mode == crate::workspace::RenameMode::Inline;
    let ws_click = cx.entity();
    let row = NavRow::new(("chat-row", chat_id), chat.title.clone())
        .icon(if flags.pinned { IconName::StarFill } else { IconName::FileText })
        .active(ix == ws.active)
        .group(format!("chat-row-{chat_id}"))
        .context_menu({
            let ws = ws_click.clone();
            move |menu, window, cx| chat_row_menu(&ws, RowMenu { id: chat_id, flags }, menu, window, cx)
        });
    if renaming {
        row.body(rename_editor(ws_click.clone(), ws.rename.clone(), chat_id))
    } else {
        row.on_click(move |ev, window, cx| {
            if ev.click_count() >= 2 {
                rename_row(&ws_click, chat_id, window, cx);
            } else {
                select_row(&ws_click, chat_id, window, cx);
            }
        })
        .suffix(row_suffix(cx.entity(), chat_id, flags, (chat.running, chat.unread)))
    }
}

/// Row click → select the chat (resolved by id — positions shift on delete).
fn select_row(ws: &Entity<Workspace>, chat_id: u64, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        if let Some(ix) = this.chat_index(chat_id) {
            this.select_chat(ix, window, cx);
        }
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

/// Trailing row content: spinner while a reply streams, unread dot, then the
/// "…" button that opens the same menu as right-click. The button stays
/// visible while its menu is up, even after the pointer leaves the row.
fn row_suffix(ws: Entity<Workspace>, chat_id: u64, flags: RowFlags, status: (bool, bool)) -> impl Fn(&mut Window, &mut App) -> AnyElement {
    let (running, unread) = status;
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
            .child(if running {
                IconName::LoaderCircle.into_any_element()
            } else if unread {
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
