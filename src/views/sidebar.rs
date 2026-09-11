use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
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
            .items_center()
            .justify_between()
            .gap_2()
            .child(div().flex().items_center().gap_2().text_sm().font_bold().child(IconName::Bot).child("Rixl Code"))
            .child(
                SidebarToggleButton::new()
                    .collapsed(collapsed)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            );

        let new_chat = SidebarMenuItem::new("New chat")
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx)));

        let items: Vec<SidebarMenuItem> = self
            .chats
            .iter()
            .enumerate()
            .rev()
            .map(|(ix, chat)| {
                let running = chat.running;
                SidebarMenuItem::new(chat.title.clone())
                    .active(ix == self.active)
                    .suffix(move |_window, _cx| if running { IconName::LoaderCircle.into_any_element() } else { div().into_any_element() })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_chat(ix, cx)))
            })
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
