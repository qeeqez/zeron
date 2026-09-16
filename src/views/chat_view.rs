use std::rc::Rc;

use crate::model::ChatMessage;
use crate::views::cards::MsgCtx;
use crate::views::render_empty_state;
use crate::views::render_message;
use crate::{EscapeKey, FindInChat, MsgNavBottom, MsgNavDown, MsgNavEnter, MsgNavTop, MsgNavUp, workspace::Workspace};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

fn chat_menu(
    menu: gpui_kit::component::menu::PopupMenu, ws: &Entity<Workspace>, pinned: bool, word_wrap: bool,
) -> gpui_kit::component::menu::PopupMenu {
    let ws_pin = ws.clone();
    let ws_rename = ws.clone();
    let ws_export = ws.clone();
    let ws_copy = ws.clone();
    let ws_wrap = ws.clone();
    let ws_snap = ws.clone();
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
    .item(PopupMenuItem::new("Snapshots").icon(IconName::Camera).on_click(move |_, _, cx| {
        ws_snap.update(cx, |this, cx| this.toggle_snapshots_panel(cx));
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
    pub fn render_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = &self.chats[self.active];
        let empty = chat.messages.is_empty();
        let messages: Rc<Vec<ChatMessage>> = chat.messages.clone();
        let running = chat.running;
        let failed = chat.failed_flag;
        let last_turn = chat.last_turn;
        let title = chat.title.clone();
        let pinned = chat.pinned;
        let ws = cx.entity();
        let ws_empty = cx.entity();
        let ws_menu = cx.entity();
        let ws_toggle = cx.entity();
        let ws_term = cx.entity();

        let running_agents = self.running_agents();
        let panel_open = self.agents_panel_open;
        let plan_open = self.plan_panel.open;
        let plan_progress = self.active_plan().map(|p| format!("{}/{}", p.done_count(), p.steps.len()));
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
        // Find bar state: matching message indices plus the current match's
        // message — the scroller rows read both for the highlight.
        let find: Option<crate::chat_find::FindMarks> = self.find.open.then(|| self.find_marks(cx));
        // Keyboard navigation cursor — the focused row's real index while the
        // transcript holds focus (see `crate::msg_nav`).
        let nav_ix = self.nav_target(window);
        let nav_focus = self.nav_focus.clone();
        let list = MessageScroller::new("chat-messages", self.scroller.clone(), move |ix, window, cx| {
            let real_ix = filtered.as_ref().map_or(ix, |f| *f.get(ix).unwrap_or(&ix));
            // Last visible message — under a filter that's the last match,
            let is_last = filtered.as_ref().map_or(real_ix == msg_count - 1, |f| ix == f.len() - 1);
            // The "Worked for Ns" label belongs to the final real message —
            // under a search filter the last match is not the turn's end.
            let duration = if !running && real_ix == msg_count - 1 { last_turn } else { None };
            let el = messages
                .get(real_ix)
                .map(|msg| render_message(MsgCtx { ix: real_ix, is_last, duration, msg }, nav_ix == Some(real_ix), &ws, window, cx))
                .unwrap_or_else(|| div().into_any_element());
            crate::chat_find::wrap_find_hit(el, real_ix, find.as_ref(), cx)
        })
        .jump_button(true)
        .with_jump_button_label("Jump to latest");

        // The header doubles as the window titlebar: it drags the window and
        // answers double-click. Interactive children stop mousedown so they
        // click instead of starting a drag.
        let header = crate::window::titlebar_drag(
            div()
                .id("chat-titlebar")
                .flex()
                .items_center()
                .gap_2()
                // Same height as the sidebar's top strip → one continuous
                // titlebar row. When the sidebar is collapsed the overlaid
                // traffic lights + toggle sit on this strip's left, so pad
                // past them.
                .h(px(crate::window::TOP_BAR_H))
                .when(self.sidebar_collapsed, |d| d.pl(px(112.)))
                .when(!self.sidebar_collapsed, |d| d.px_4())
                .pr_4()
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
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            ws_toggle.update(cx, |this, cx| this.toggle_agents_panel(cx));
                        }),
                )
                .child(crate::views::plan_panel::plan_toggle(plan_open, plan_progress, cx))
                .child(
                    div()
                        .id("terminal-toggle")
                        .test_support()
                        .flex()
                        .items_center()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .when(self.terminal.open, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
                        .when(!self.terminal.open, |d| d.text_color(cx.theme().muted_foreground))
                        .child(IconName::SquareTerminal)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            ws_term.update(cx, |this, cx| this.toggle_terminal(window, cx));
                        }),
                )
                .child(Button::new("chat-menu").ghost().icon(IconName::Ellipsis).dropdown_menu({
                    let word_wrap = self.word_wrap;
                    move |menu, _window, _cx| chat_menu(menu, &ws_menu, pinned, word_wrap)
                })),
        );

        // The shared top bar in `Workspace::render` covers the window's drag
        // strip + sidebar toggle; the content pane starts below it.

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(cx.theme().background)
            // FindInChat lives here rather than on the workspace root so the
            // bar stays scoped to the chat pane; EscapeKey intercepts Esc to
            // close the bar before the workspace's own Esc handling runs.
            .on_action(cx.listener(|this, _: &FindInChat, window, cx| this.open_chat_find(window, cx)))
            .on_action(cx.listener(|this, _: &EscapeKey, window, cx| this.find_escape(window, cx)))
            // Message navigation: j/k/↑/↓/gg/G/Enter reach here only when no
            // input owns the keys (see `msg_nav::nav_keys_allowed`); Esc exits
            // nav — or enters it from an idle composer — before the
            // workspace's own Esc handling.
            .on_action(cx.listener(|this, _: &MsgNavDown, window, cx| this.nav_move(false, window, cx)))
            .on_action(cx.listener(|this, _: &MsgNavUp, window, cx| this.nav_move(true, window, cx)))
            .on_action(cx.listener(|this, _: &MsgNavTop, window, cx| this.nav_g(window, cx)))
            .on_action(cx.listener(|this, _: &MsgNavBottom, window, cx| this.nav_bottom(window, cx)))
            .on_action(cx.listener(|this, _: &MsgNavEnter, window, cx| this.nav_activate(window, cx)))
            .on_action(cx.listener(|this, _: &EscapeKey, window, cx| this.nav_escape(window, cx)))
            // The code-block Apply button dispatches here — the chat pane
            // owns it so the write lands on the chat the block belongs to.
            .on_action(cx.listener(|this, action: &crate::apply_code::ApplyCodeBlock, window, cx| {
                this.apply_code_block(action.code.clone(), action.lang.clone(), window, cx);
            }))
            .child(header)
            // Restricted-mode notice for an untrusted folder — the "Trust…"
            // button reopens the trust dialog (see `crate::views::trust`).
            .when(!self.trusted, |d| d.child(crate::views::trust::restricted_banner(cx)))
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
            .when(self.find.open, |d| d.child(self.find_bar(cx)))
            .child(
                div()
                    .id("msg-nav")
                    .test_support()
                    .track_focus(&nav_focus)
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(MouseButton::Left, cx.listener(Workspace::nav_click))
                    .child(if empty {
                        render_empty_state(ws_empty.clone(), self.project.root(), cx).into_any_element()
                    } else {
                        list.into_any_element()
                    }),
            )
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
