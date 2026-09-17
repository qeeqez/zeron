//! The message row's right-click menu: copy variants on top, then quote,
//! bookmark, fork/split, view-raw, edit, and the retry/regenerate tail.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Role;
use crate::views::cards::MsgCtx;
use crate::views::markdown::MarkdownState;
use crate::workspace::Workspace;

/// Render-time gates `msg_menu` needs — bundled to stay under the
/// arg-count lint. The caller computes them from the live chat so the
/// items reflect the state at render time.
pub(super) struct MenuGates {
    /// "Undo turn" applies to this message.
    pub undoable: bool,
    /// A turn is in flight — "Fork from here" stays listed but inert.
    pub running: bool,
    /// The transcript's last message — forking it would just duplicate.
    pub last_msg: bool,
}

/// Build the context menu for message `mc.ix`. `source` is the message's
/// raw text — cloned cheap (SharedString) and scanned for fenced blocks
/// only when the menu actually opens. `mc` is borrowed, not moved: the
/// returned closure must be `'static`, so the fields it needs are copied
/// out.
pub(super) fn msg_menu(
    mc: &MsgCtx, ws: Entity<Workspace>, md_state: Option<Entity<MarkdownState>>, source: SharedString, gates: MenuGates,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + use<> {
    let MenuGates { undoable, running, last_msg } = gates;
    let ix = mc.ix;
    let is_last = mc.is_last;
    let role = mc.msg.role;
    let bookmarked = mc.msg.bookmarked;
    let is_error = mc.msg.is_error();
    move |menu, window, cx| {
        // Copy variants stay grouped at the top; Copy Code only appears
        // when the message actually has fenced blocks.
        let menu = menu
            .item(msg_item("Copy", IconName::Copy, &ws, move |this, _w, cx| this.copy_message(ix, cx)))
            .item(msg_item("Copy as Markdown", IconName::FileCode, &ws, move |this, _w, cx| {
                this.copy_message_markdown(ix, cx)
            }))
            .when(!crate::chat_msg::copy::code_blocks(&source).is_empty(), |menu| {
                menu.item(msg_item("Copy Code", IconName::SquareCode, &ws, move |this, _w, cx| {
                    this.copy_message_code(ix, cx)
                }))
            })
            .item(msg_item("Quote", IconName::Quote, &ws, move |this, w, cx| this.quote_message(ix, w, cx)))
            // "Quote selection" appears only while this message's body has
            // an active selection — the text is captured as the menu opens
            // because the item's own click would clear it first.
            .when_some(md_state.as_ref().map(|md| md.read(cx).view.read(cx).selected_text()).filter(|s| !s.trim().is_empty()), |menu, selected| {
                menu.item(msg_item("Quote selection", IconName::Quote, &ws, move |this, w, cx| {
                    this.quote_selection(&selected, w, cx)
                }))
            })
            .item(msg_item(if bookmarked { "Remove bookmark" } else { "Bookmark" }, IconName::Star, &ws, move |this, _w, cx| {
                this.toggle_bookmark(ix, cx)
            }))
            .separator();
        // "Fork from here" is hidden on the last message — forking the
        // full transcript is just "Fork chat" from the titlebar menu —
        // and disabled while a turn runs.
        let menu = if last_msg {
            menu
        } else {
            menu.item(
                msg_item("Fork from here", IconName::GitFork, &ws, move |this, w, cx| this.fork_chat(this.active, Some(ix), w, cx))
                    .disabled(running),
            )
        };
        // Splitting at the first message leaves nothing behind — the
        // item only exists where a prefix would remain.
        let menu = if ix > 0 {
            menu.item(msg_item("Split chat here", IconName::Scissors, &ws, move |this, w, cx| {
                let id = this.chats[this.active].id;
                this.split_chat(id, ix, w, cx)
            }))
        } else {
            menu
        };
        let menu = if let Some(md) = md_state.clone() {
            let label = if md.read(cx).raw { "View rendered" } else { "View raw" };
            menu.item(
                PopupMenuItem::new(label)
                    .icon(IconName::Code)
                    .on_click(super::message_footer::toggle_raw(md, ws.clone(), ix)),
            )
        } else if undoable {
            menu.item(msg_item("Undo turn", IconName::Undo2, &ws, move |this, w, cx| this.undo_turn(ix, w, cx)))
        } else {
            menu
        };
        let menu = if role == Role::User {
            menu.item(msg_item("Edit", IconName::Pencil, &ws, move |this, w, cx| this.edit_message(ix, w, cx)))
        } else {
            menu
        };
        match (role, is_last) {
            // A failed turn's row swaps the retry tail for "Retry turn" —
            // same resend path as Regenerate, labeled for the failure.
            (Role::Assistant, _) if is_error => {
                let menu = menu.item(
                    msg_item("Retry turn", IconName::RotateCcw, &ws, move |this, w, cx| this.regenerate_from(ix, w, cx)).disabled(running),
                );
                if is_last { super::retry_menu::retry_model_submenu(menu, &ws, window, cx) } else { menu }
            },
            (Role::Assistant, true) => super::retry_menu::retry_items(menu, &ws, window, cx),
            (Role::Assistant, false) => {
                menu.item(msg_item("Regenerate", IconName::RotateCcw, &ws, move |this, w, cx| this.regenerate_from(ix, w, cx)))
            },
            _ => menu,
        }
    }
}

/// One context-menu item that runs a `Workspace` method on click.
fn msg_item(
    label: &'static str, icon: IconName, ws: &Entity<Workspace>, f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> PopupMenuItem {
    let ws = ws.clone();
    PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
        ws.update(cx, |this, cx| f(this, window, cx));
    })
}
