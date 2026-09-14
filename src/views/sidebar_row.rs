//! Chat rows in the sidebar. `ChatRow` is a custom `SidebarItem` (rather than
//! `SidebarMenuItem`) because Codex-style chat management needs two things the
//! stock item can't express: a hover-revealed "…" menu button and an inline
//! rename editor that replaces the title label.

mod sidebar_menu;

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu};
use gpui_kit::component::sidebar::{SidebarItem, SidebarMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Collapsible, Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::workspace::Workspace;

use sidebar_menu::{RowFlags, chat_row_menu};

/// A sidebar row: either a chat row or a plain action item ("New chat").
/// `Sidebar`/`SidebarGroup` are homogeneous, so both shapes share this type.
/// `Item` is boxed — `SidebarMenuItem` is ~2KB and would dwarf `Chat`.
#[derive(Clone)]
pub(super) enum SidebarRow {
    Chat(ChatRow),
    Item(Box<SidebarMenuItem>),
}

impl Collapsible for SidebarRow {
    fn is_collapsed(&self) -> bool {
        match self {
            Self::Chat(row) => row.is_collapsed(),
            Self::Item(item) => item.is_collapsed(),
        }
    }

    fn collapsed(self, collapsed: bool) -> Self {
        match self {
            Self::Chat(row) => Self::Chat(row.collapsed(collapsed)),
            Self::Item(item) => Self::Item(Box::new(item.collapsed(collapsed))),
        }
    }
}

impl SidebarItem for SidebarRow {
    fn render(self, id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> impl IntoElement {
        match self {
            Self::Chat(row) => row.render(id, window, cx).into_any_element(),
            Self::Item(item) => (*item).render(id, window, cx).into_any_element(),
        }
    }
}

/// One chat row: icon + title (or inline rename editor) + status + "…" menu.
#[derive(Clone)]
pub(super) struct ChatRow {
    ws: Entity<Workspace>,
    rename_input: Entity<InputState>,
    chat_id: u64,
    title: SharedString,
    flags: RowFlags,
    running: bool,
    unread: bool,
    active: bool,
    renaming: bool,
    collapsed: bool,
}

pub(super) fn chat_row(chat: &Chat, ix: usize, ws: &Workspace, cx: &mut Context<Workspace>) -> ChatRow {
    ChatRow {
        ws: cx.entity(),
        rename_input: ws.rename.clone(),
        chat_id: chat.id,
        title: chat.title.clone(),
        flags: RowFlags {
            pinned: chat.pinned,
            archived: chat.archived,
            only_chat: ws.chats.len() <= 1,
        },
        running: chat.running,
        unread: chat.unread,
        active: ix == ws.active,
        // Only an inline rename mounts the editor — a dialog rename shares
        // `ws.rename`, and its outside-click would commit behind the dialog.
        renaming: ws.renaming == Some(chat.id) && ws.rename_mode == crate::workspace::RenameMode::Inline,
        collapsed: false,
    }
}

impl Collapsible for ChatRow {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl SidebarItem for ChatRow {
    fn render(self, id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = id.into();
        let chat_id = self.chat_id;
        let flags = self.flags;
        let group = SharedString::from(format!("chat-row-{chat_id}"));
        // Tracks whether this row's "…" dropdown is open — the button stays
        // visible while its menu is up, even after the pointer leaves the row.
        let menu_open = window.use_keyed_state((id.clone(), "menu-open"), cx, |_, _| false);
        let ws_click = self.ws.clone();
        let ws_menu = self.ws.clone();
        div().id(id).test_support().w_full().group(group).child(
            h_flex()
                .size_full()
                .id(("chat-row", chat_id))
                .test_support()
                .overflow_x_hidden()
                .flex_shrink_0()
                .p_2()
                .gap_x_2()
                .rounded(cx.theme().radius)
                .text_sm()
                .when(!self.active, |this| {
                    this.hover(|this| this.bg(cx.theme().sidebar_accent.opacity(0.8)).text_color(cx.theme().sidebar_accent_foreground))
                })
                .when(self.active, |this| {
                    this.font_medium()
                        .bg(cx.theme().tokens.sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
                .when_some(self.icon(), |this, icon| this.child(icon))
                .when(self.collapsed, |this| this.justify_center())
                .when(!self.collapsed, |this| this.h_7().child(self.body(&menu_open, cx)))
                .when(!self.renaming, |this| this.on_click(move |_, window, cx| select_row(&ws_click, chat_id, window, cx)))
                .context_menu(move |menu, _window, _cx| chat_row_menu(&ws_menu, chat_id, flags, menu)),
        )
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

impl ChatRow {
    fn icon(&self) -> Option<IconName> {
        Some(if self.flags.pinned { IconName::StarFill } else { IconName::FileText })
    }

    /// Title label or, while renaming, the inline editor; plus the trailing
    /// status indicator and hover-revealed "…" menu button.
    fn body(&self, menu_open: &Entity<bool>, cx: &App) -> impl IntoElement {
        h_flex()
            .flex_1()
            .gap_x_2()
            .justify_between()
            .overflow_x_hidden()
            .when(self.renaming, |this| this.child(self.rename_editor()))
            .when(!self.renaming, |this| {
                this.child(h_flex().flex_1().overflow_x_hidden().child(self.title.clone()))
                    .child(self.suffix(menu_open, cx))
            })
    }

    /// Inline title editor — Enter commits, Escape cancels, and a mouse-down
    /// anywhere outside the field commits (Finder-style).
    fn rename_editor(&self) -> impl IntoElement {
        let ws_out = self.ws.clone();
        let ws_enter = self.ws.clone();
        let ws_esc = self.ws.clone();
        div()
            .flex_1()
            .min_w_0()
            .on_mouse_down_out(move |_, window, cx| {
                ws_out.update(cx, |this, cx| this.commit_rename(window, cx));
            })
            .on_action(move |_: &InputEnter, window, cx| {
                cx.stop_propagation();
                ws_enter.update(cx, |this, cx| this.commit_rename(window, cx));
            })
            .on_action(move |_: &InputEscape, window, cx| {
                cx.stop_propagation();
                ws_esc.update(cx, |this, cx| this.cancel_inline_rename(window, cx));
            })
            .child(Input::new(&self.rename_input).id(("rename-input", self.chat_id)).xsmall().w_full())
    }

    /// Trailing row content: spinner while a reply streams, unread dot, then
    /// the "…" button that opens the same menu as right-click.
    fn suffix(&self, menu_open: &Entity<bool>, cx: &App) -> impl IntoElement {
        let group = SharedString::from(format!("chat-row-{}", self.chat_id));
        let ws = self.ws.clone();
        let chat_id = self.chat_id;
        let flags = self.flags;
        h_flex()
            .items_center()
            .gap_1()
            .child(if self.running {
                IconName::LoaderCircle.into_any_element()
            } else if self.unread {
                div().w_2().h_2().rounded_full().bg(hsla(0.0, 0.0, 0.55, 1.0)).into_any_element()
            } else {
                div().into_any_element()
            })
            .child(
                div()
                    .id(("chat-menu", chat_id))
                    .test_support()
                    .when(!menu_open.read(cx), |this| this.invisible().group_hover(group, |style| style.visible()))
                    .child(
                        Button::new(("chat-menu-btn", chat_id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _window, _cx| chat_row_menu(&ws, chat_id, flags, menu))
                            .on_open_change({
                                let menu_open = menu_open.clone();
                                move |open, _window, cx| {
                                    menu_open.update(cx, |state, _| *state = *open);
                                }
                            }),
                    ),
            )
    }
}
