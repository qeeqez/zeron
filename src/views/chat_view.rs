use std::rc::Rc;

use crate::model::ChatMessage;
use crate::views::cards::MsgCtx;
use crate::views::chat_menu::chat_menu;
use crate::views::render_empty_state;
use crate::views::render_message;
use crate::{EscapeKey, FindInChat, MsgNavBottom, MsgNavDown, MsgNavEnter, MsgNavTop, MsgNavUp, workspace::Workspace};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};

use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Workspace {
    pub fn render_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = &self.chats[self.active];
        let empty = chat.messages.is_empty();
        let messages: Rc<Vec<ChatMessage>> = chat.messages.clone();
        let running = chat.running;
        let failed = chat.failed_flag;
        // A live rate limit replaces the generic failure row — its banner
        // carries the same Retry affordance plus the reset time.
        let limited = chat.usage.rate_limit.as_ref().is_some_and(|rl| rl.limited);
        let last_turn = chat.last_turn;
        let title = chat.title.clone();
        let pinned = chat.pinned;
        // Worktree threads get a titlebar chip + the ⋯ menu's reveal/open
        // items; the chip's tooltip carries the checkout path.
        let worktree = chat.worktree;
        let workdir = chat.workdir.clone();
        // Temporary chats get a muted "Temporary" chip next to the title —
        // a chat can be both worktree and ephemeral, so both chips render.
        let ephemeral = chat.ephemeral;
        // A color tag shows as a small dot beside the title — same slot as
        // the worktree/temp chips; all three can coexist.
        let color = chat.color;
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
        let filtered: Option<Vec<usize>> =
            (!query.is_empty()).then(|| (0..msg_count).filter(|&ix| crate::chat_search::msg_matches(&messages[ix], &query)).collect());
        // Rows the scroller shows — the pill's "N new" counts arrivals
        // after this snapshot while the transcript is scrolled up.
        let visible_count = filtered.as_ref().map_or(msg_count, Vec::len);
        crate::chat_search::update_pill_anchor(&self.scroller, &mut self.pill_anchor, visible_count, cx);
        let unseen = visible_count.saturating_sub(self.pill_anchor.unwrap_or(visible_count));
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
            // Day separator above the first visible message of each new day —
            // under a filter the previous *match* supplies the boundary.
            let prev_at = match &filtered {
                None => real_ix.checked_sub(1).and_then(|p| messages.get(p)).map(|m| m.at),
                Some(f) => ix.checked_sub(1).and_then(|p| f.get(p)).and_then(|&p| messages.get(p)).map(|m| m.at),
            };
            let at = messages.get(real_ix).map(|m| m.at);
            let el = messages
                .get(real_ix)
                .map(|msg| render_message(MsgCtx { ix: real_ix, is_last, duration, msg }, nav_ix == Some(real_ix), &ws, window, cx))
                .unwrap_or_else(|| div().into_any_element());
            let el = crate::chat_find::wrap_find_hit(el, real_ix, find.as_ref(), cx);
            crate::views::date_separator::separator_row(real_ix, at, prev_at, el, cx)
        })
        .with_jump_button_renderer(crate::chat_search::pill_renderer(ws_empty.clone(), unseen));

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
                .when_some(color, |d, color| d.child(crate::views::chat_menu::color_dot("chat-color-dot", color, px(8.))))
                .when(worktree, |d| d.child(crate::views::chat_menu::worktree_badge("worktree-badge", &workdir, cx)))
                .when(ephemeral, |d| d.child(crate::views::chat_menu::temp_badge("temp-badge", cx)))
                .when(chat.instructions.is_some(), |d| d.child(crate::chat_ops::instructions::instructions_badge("instructions-badge", cx)))
                // Context-window meter — the latest usage report's occupancy,
                // or cumulative tokens when the backend reports no window.
                .when_some(crate::views::chat_menu::context_chip("context-meter", &chat.usage, cx), |d, chip| d.child(chip))
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
                    let can_split = msg_count >= 2 && !running;
                    let state = crate::views::chat_menu::ChatMenuState { pinned, word_wrap, color, worktree, ephemeral, can_split, running };
                    move |menu, window, cx| chat_menu(menu, &ws_menu, state, window, cx)
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
            // File drop covers the whole chat pane — messages included.
            // `attach_incoming` routes images to chips and other files to
            // @-mentions; `drag_over` tints the pane while files hover.
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().tokens.drop_target))
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.attach_incoming(paths.0.to_vec(), window, cx);
            }))
            .child(header)
            // Restricted-mode notice for an untrusted folder — the "Trust…"
            // button reopens the trust dialog (see `crate::views::trust`).
            .when(!self.trusted, |d| d.child(crate::views::trust::restricted_banner(cx)))
            .when(self.chat_search_open, |d| d.child(self.chat_search_bar(cx)))
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
                        render_empty_state(ws_empty.clone(), self, cx).into_any_element()
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
            .when_some(
                chat.usage.rate_limit.as_ref().and_then(|rl| crate::views::rate_limit::rate_limit_banner(rl, running, &ws_empty, cx)),
                |d, banner| d.child(banner),
            )
            .when_some(self.budget_alert_visible(chat).map(|(spent, cap)| crate::views::budget::budget_banner(spent, cap, &ws_empty, cx)), |d, b| {
                d.child(b)
            })
            .when(failed && !running && !limited, |d| {
                let ws_retry = ws_empty.clone();
                let row = d.child(
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
                );
                // A second model to switch to earns the banner its own
                // picker — a pick selects the model then retries.
                if self.available_models().len() < 2 {
                    return row;
                }
                row.child(crate::views::model_picker(crate::views::ModelPickerSpec {
                    current_provider: self.selected_provider.clone(),
                    current_model: self.model.clone(),
                    providers: self
                        .enabled_providers()
                        .into_iter()
                        .map(|p| crate::views::PickerProvider {
                            id: p.id.clone(),
                            name: p.name.clone(),
                            icon: p.kind.info().icon,
                            models: self.models_for(&p.id),
                        })
                        .collect(),
                    ws: ws_empty.clone(),
                    on_pick: Some(Workspace::retry_last),
                }))
            })
            .child(self.render_composer(cx))
    }
}
