use std::rc::Rc;

use crate::model::ChatMessage;
use crate::views::cards::MsgCtx;
use crate::views::render_empty_state;
use crate::views::render_message;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

fn chat_menu(
    menu: gpui_kit::component::menu::PopupMenu, ws: &Entity<Workspace>, pinned: bool, word_wrap: bool,
) -> gpui_kit::component::menu::PopupMenu {
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_export = ws.clone();
    let ws_copy = ws.clone();
    let ws_wrap = ws.clone();
    menu.item(
        PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" })
            .icon(IconName::Star)
            .on_click(move |_, _, cx| {
                ws_pin.update(cx, |this, cx| this.toggle_pin(this.active, cx));
            }),
    )
    .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, window, cx| {
        ws_rename.update(cx, |this, cx| this.rename_active(window, cx));
    }))
    .item(PopupMenuItem::new("Export").icon(IconName::Share).on_click(move |_, _, cx| {
        ws_export.update(cx, |this, cx| this.export_active(cx));
    }))
    .item(PopupMenuItem::new("Copy transcript").icon(IconName::Copy).on_click(move |_, _, cx| {
        ws_copy.update(cx, |this, cx| this.copy_transcript(cx));
    }))
    .item(PopupMenuItem::new("Word wrap").icon(IconName::Check).checked(word_wrap).on_click(move |_, _, cx| {
        ws_wrap.update(cx, |this, cx| {
            this.word_wrap = !this.word_wrap;
            this.save_settings();
            // Every message's height changed — remeasure the whole list.
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
            cx.notify();
        });
    }))
}

impl Workspace {
    pub fn render_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = &self.chats[self.active];
        let empty = chat.messages.is_empty();
        let messages: Rc<Vec<ChatMessage>> = chat.messages.clone();
        let running = chat.running;
        let failed = chat.failed_flag;
        let title = chat.title.clone();
        let pinned = chat.pinned;
        let ws = cx.entity();
        let ws_empty = cx.entity();
        let ws_menu = cx.entity();
        let ws_toggle = cx.entity();

        let running_agents = self.running_agents();
        let panel_open = self.agents_panel_open;

        let msg_count = messages.len();
        let query = if self.chat_search_open {
            self.chat_search.read(cx).value().to_string().to_lowercase()
        } else {
            String::new()
        };
        // None = unfiltered — avoids allocating 0..n every render.
        let filtered: Option<Vec<usize>> = if query.is_empty() {
            None
        } else {
            Some((0..msg_count).filter(|&ix| crate::chat_search::msg_matches(&messages[ix], &query)).collect())
        };
        let list = MessageScroller::new("chat-messages", self.scroller.clone(), move |ix, _window, cx| {
            let real_ix = filtered.as_ref().map_or(ix, |f| *f.get(ix).unwrap_or(&ix));
            messages
                .get(real_ix)
                .map(|msg| render_message(MsgCtx { ix: real_ix, is_last: real_ix == msg_count - 1 }, msg, &ws, cx))
                .unwrap_or_else(|| div().into_any_element())
        })
        .jump_button(true)
        .with_jump_button_label("Jump to latest");

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child(title)
            .child(div().flex_1())
            .child(
                div()
                    .id("agents-toggle")
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .when(panel_open, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
                    .when(!panel_open, |d| d.text_color(cx.theme().muted_foreground))
                    .child(IconName::Bot)
                    .when(running_agents > 0, |d| d.child(format!("{running_agents}")))
                    .on_click(move |_, _, cx| {
                        ws_toggle.update(cx, |this, cx| this.toggle_agents_panel(cx));
                    }),
            )
            .child(Button::new("chat-menu").ghost().icon(IconName::Ellipsis).dropdown_menu({
                let word_wrap = self.word_wrap;
                move |menu, _window, _cx| chat_menu(menu, &ws_menu, pinned, word_wrap)
            }));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(cx.theme().background)
            .child(header)
            .when(self.chat_search_open, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(IconName::Search)
                        .child(div().flex_1().child(Input::new(&self.chat_search).appearance(true)))
                        .child(
                            div()
                                .id("close-search")
                                .cursor_pointer()
                                .text_color(cx.theme().muted_foreground)
                                .child(IconName::X)
                                .on_click(cx.listener(|this, _, window, cx| this.open_chat_search(window, cx))),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(if empty {
                render_empty_state(ws_empty.clone(), cx).into_any_element()
            } else {
                list.into_any_element()
            }))
            .when(running, |d| {
                let elapsed = chat.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::LoaderCircle)
                        .child(format!("Working… {elapsed}s")),
                )
            })
            .when(failed && !running, |d| {
                let ws_retry = ws_empty.clone();
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(IconName::TriangleAlert)
                        .child("Reply failed")
                        .child(div().id("retry-failed").cursor_pointer().underline().child("Retry").on_click(move |_, _, cx| {
                            ws_retry.update(cx, |this, cx| this.retry_last(cx));
                        })),
                )
            })
            .child(self.render_composer(cx))
    }
}
