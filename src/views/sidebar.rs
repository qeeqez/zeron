use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenuItem, SidebarToggleButton};

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let header = div()
            .flex()
            .flex_col()
            .gap_2()
            // Clear the floating traffic lights.
            .pt(px(28.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex().items_center().gap_2().text_sm().font_bold().child(IconName::Bot).child("Rixl Code"))
                    .child(
                        SidebarToggleButton::new()
                            .collapsed(collapsed)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1().child(Input::new(&self.search).prefix(IconName::Search).appearance(true)))
                    .when(!self.search.read(cx).value().is_empty(), |d| {
                        d.child(
                            div()
                                .id("search-clear")
                                .cursor_pointer()
                                .text_color(cx.theme().muted_foreground)
                                .child(IconName::X)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.search.update(cx, |s, cx| s.set_value("", window, cx));
                                })),
                        )
                    }),
            );

        let new_chat = SidebarMenuItem::new("New chat")
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx)));

        let query = self.search.read(cx).value().to_lowercase();
        let filtered = self.sidebar_order(&query);
        let archived: Vec<usize> = (0..self.chats.len())
            .filter(|ix| self.chats[*ix].archived && (query.is_empty() || self.chats[*ix].title.to_lowercase().contains(&query)))
            .collect();
        let bucket = |ix: usize| self.chat_bucket(ix);

        let group_names = ["Pinned", "Today", "Previous 7 Days", "Older"];
        let mut groups: Vec<SidebarGroup<SidebarMenuItem>> = Vec::new();
        for (bucket_ix, name) in group_names.iter().enumerate() {
            let items: Vec<SidebarMenuItem> = filtered
                .iter()
                .copied()
                .filter(|ix| bucket(*ix) == bucket_ix)
                .map(|ix| chat_row(&self.chats[ix], ix, self.active, cx))
                .collect();
            if !items.is_empty() {
                groups.push(SidebarGroup::new(*name).children(items));
            }
        }
        if !archived.is_empty() {
            let items: Vec<SidebarMenuItem> = archived.iter().copied().map(|ix| chat_row(&self.chats[ix], ix, self.active, cx)).collect();
            groups.push(SidebarGroup::new("Archived").children(items));
        }

        let actions = SidebarGroup::new("").child(new_chat);

        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(IconName::CircleUser)
            .child("Local")
            .child(div().flex_1())
            .child(
                div()
                    .id("clear-chats")
                    .cursor_pointer()
                    .child(IconName::Trash)
                    .on_click(cx.listener(|this, _, window, cx| this.clear_all_chats(window, cx))),
            )
            .child(
                div()
                    .id("settings-btn")
                    .cursor_pointer()
                    .child(IconName::Settings)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(window, cx);
                    })),
            );

        div()
            .id("sidebar-wrap")
            .h_full()
            .relative()
            .bg(cx.theme().sidebar.opacity(0.6))
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                Sidebar::new("sidebar")
                    .w(px(self.sidebar_width))
                    .collapsible(SidebarCollapsible::Icon)
                    .collapsed(collapsed)
                    .header(header)
                    .child(actions)
                    .children(groups)
                    .footer(footer),
            )
            .when(!collapsed, |d| {
                d.child(
                    div()
                        .id("sidebar-resize")
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .w(px(5.))
                        .cursor_col_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.resizing_sidebar = true;
                                cx.notify();
                            }),
                        ),
                )
            })
    }
}

fn chat_row(chat: &crate::model::Chat, ix: usize, active: usize, cx: &mut Context<Workspace>) -> SidebarMenuItem {
    let running = chat.running;
    let pinned = chat.pinned;
    let unread = chat.unread;
    let archived = chat.archived;
    let ws = cx.entity();
    SidebarMenuItem::new(chat.title.clone())
        .active(ix == active)
        .icon(if pinned { IconName::StarFill } else { IconName::FileText })
        .suffix(move |_window, _cx| {
            if running {
                IconName::LoaderCircle.into_any_element()
            } else if unread {
                div().w_2().h_2().rounded_full().bg(hsla(0.0, 0.0, 0.55, 1.0)).into_any_element()
            } else {
                div().into_any_element()
            }
        })
        .context_menu(move |menu, _window, _cx| chat_row_menu(&ws, ix, RowFlags { pinned, archived }, menu))
        .on_click(cx.listener(move |this, _, window, cx| this.select_chat(ix, window, cx)))
}

/// Per-row state the context menu needs.
struct RowFlags {
    pinned: bool,
    archived: bool,
}

fn chat_row_menu(ws: &Entity<Workspace>, ix: usize, flags: RowFlags, menu: PopupMenu) -> PopupMenu {
    let RowFlags { pinned, archived } = flags;
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    menu.item(PopupMenuItem::new(pin_label).icon(IconName::Star).on_click(move |_, _, cx| {
        ws_pin.update(cx, |this, cx| this.toggle_pin(ix, cx));
    }))
    .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, window, cx| {
        ws_rename.update(cx, |this, cx| this.open_rename(ix, window, cx));
    }))
    .item(PopupMenuItem::new("Duplicate").icon(IconName::Copy).on_click({
        let ws = ws.clone();
        move |_, window, cx| {
            ws.update(cx, |this, cx| this.duplicate_chat(ix, window, cx));
        }
    }))
    .item(PopupMenuItem::new("Export").icon(IconName::Share).on_click({
        let ws = ws.clone();
        move |_, _, cx| {
            ws.update(cx, |this, cx| this.export_chat(ix, cx));
        }
    }))
    .item(PopupMenuItem::new("Delete").icon(IconName::Delete).on_click({
        let ws = ws.clone();
        move |_, window, cx| {
            ws.update(cx, |this, cx| this.delete_chat(ix, window, cx));
        }
    }))
    .item(
        PopupMenuItem::new(if archived { "Unarchive" } else { "Archive" })
            .icon(IconName::Archive)
            .on_click({
                let ws = ws.clone();
                move |_, window, cx| {
                    ws.update(cx, |this, cx| this.toggle_archive(ix, window, cx));
                }
            }),
    )
}
