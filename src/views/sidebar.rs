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
            .child(Input::new(&self.search).prefix(IconName::Search).appearance(true));

        let new_chat = SidebarMenuItem::new("New chat")
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx)));

        let query = self.search.read(cx).value().to_lowercase();
        let mut order: Vec<usize> = (0..self.chats.len()).collect();
        order.sort_by_key(|ix| !self.chats[*ix].pinned);
        let items: Vec<SidebarMenuItem> = order
            .into_iter()
            .rev()
            .filter(|ix| query.is_empty() || self.chats[*ix].title.to_lowercase().contains(&query))
            .map(|ix| chat_row(&self.chats[ix], ix, self.active, cx))
            .collect();

        let actions = SidebarGroup::new("").child(new_chat);
        let chats_group = SidebarGroup::new("Chats").children(items);

        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(IconName::CircleUser)
            .child("Local")
            .child(div().flex_1())
            .child(IconName::Settings);

        div()
            .id("sidebar-wrap")
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                Sidebar::new("sidebar")
                    .collapsible(SidebarCollapsible::Icon)
                    .collapsed(collapsed)
                    .header(header)
                    .child(actions)
                    .child(chats_group)
                    .footer(footer),
            )
    }
}

fn chat_row(chat: &crate::model::Chat, ix: usize, active: usize, cx: &mut Context<Workspace>) -> SidebarMenuItem {
    let running = chat.running;
    let pinned = chat.pinned;
    let ws = cx.entity();
    SidebarMenuItem::new(chat.title.clone())
        .active(ix == active)
        .icon(if pinned { IconName::StarFill } else { IconName::FileText })
        .suffix(move |_window, _cx| if running { IconName::LoaderCircle.into_any_element() } else { div().into_any_element() })
        .context_menu(move |menu, _window, _cx| chat_row_menu(&ws, ix, pinned, menu))
        .on_click(cx.listener(move |this, _, _, cx| this.select_chat(ix, cx)))
}

fn chat_row_menu(ws: &Entity<Workspace>, ix: usize, pinned: bool, menu: PopupMenu) -> PopupMenu {
    let ws_pin = ws.clone();
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    menu.item(PopupMenuItem::new(pin_label).icon(IconName::Star).on_click(move |_, _, cx| {
        ws_pin.update(cx, |this, cx| this.toggle_pin(ix, cx));
    }))
    .separator()
    .item(PopupMenuItem::new("Delete").icon(IconName::Delete).on_click({
        let ws = ws.clone();
        move |_, _, cx| {
            ws.update(cx, |this, cx| this.delete_chat(ix, cx));
        }
    }))
}
