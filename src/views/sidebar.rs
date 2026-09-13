use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenuItem};

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
            // Top strip: clears the floating traffic lights + toggle and drags
            // the window (the app owns titlebar dragging).
            .child(crate::window::titlebar_drag(div().id("sidebar-titlebar").h(px(28.))))
            .child(div().flex().items_center().gap_2().text_sm().font_bold().child(IconName::Bot).child("Rixl Code"))
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
                .map(|ix| super::sidebar_row::chat_row(&self.chats[ix], ix, self.active, cx))
                .collect();
            if !items.is_empty() {
                groups.push(SidebarGroup::new(*name).children(items));
            }
        }
        if !archived.is_empty() {
            let items: Vec<SidebarMenuItem> = archived
                .iter()
                .copied()
                .map(|ix| super::sidebar_row::chat_row(&self.chats[ix], ix, self.active, cx))
                .collect();
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
                    .test_support()
                    .cursor_pointer()
                    .child(IconName::Settings)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(window, cx);
                    })),
            );

        div()
            .id("sidebar-wrap")
            .test_support()
            .h_full()
            .relative()
            .bg(cx.theme().sidebar.opacity(0.6))
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                Sidebar::new("sidebar")
                    .w(px(self.sidebar_width))
                    .collapsible(SidebarCollapsible::Offcanvas)
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
