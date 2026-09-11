use std::time::{Duration, SystemTime};

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenuItem, SidebarToggleButton};

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::theme::{ActiveTheme, Theme, ThemeMode};
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
        let now = SystemTime::now();
        let day = Duration::from_secs(86_400);
        let bucket = |ix: usize| {
            let chat = &self.chats[ix];
            if chat.pinned {
                return 0;
            }
            match now.duration_since(chat.created_at) {
                Ok(d) if d < day => 1,
                Ok(d) if d < day * 7 => 2,
                _ => 3,
            }
        };
        let mut order: Vec<usize> = (0..self.chats.len()).collect();
        order.sort_by_key(|ix| (bucket(*ix), std::cmp::Reverse(self.chats[*ix].created_at)));
        let filtered: Vec<usize> = order
            .into_iter()
            .filter(|ix| query.is_empty() || self.chats[*ix].title.to_lowercase().contains(&query))
            .collect();

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

        let actions = SidebarGroup::new("").child(new_chat);

        let footer =
            div()
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
                        .on_click(cx.listener(|this, _, _, cx| this.clear_all_chats(cx))),
                )
                .child(div().id("settings-btn").cursor_pointer().child(IconName::Settings).on_click(cx.listener(
                    |_this, _, window, cx| {
                        window.open_sheet(cx, |sheet, _window, _cx| sheet.title("Settings").child(settings_body()));
                    },
                )));

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
                    .children(groups)
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
        move |_, _, cx| {
            ws.update(cx, |this, cx| this.duplicate_chat(ix, cx));
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
        move |_, _, cx| {
            ws.update(cx, |this, cx| this.delete_chat(ix, cx));
        }
    }))
}

fn settings_body() -> impl IntoElement {
    div().flex().flex_col().gap_2().p_4().child(div().text_sm().child("Theme")).child(
        div()
            .flex()
            .gap_2()
            .child(theme_button("Light", ThemeMode::Light))
            .child(theme_button("Dark", ThemeMode::Dark)),
    )
}

fn theme_button(label: &'static str, mode: ThemeMode) -> impl IntoElement {
    Button::new(SharedString::from(label)).outline().label(label).on_click(move |_, _, cx| {
        Theme::change(mode, None, cx);
    })
}
