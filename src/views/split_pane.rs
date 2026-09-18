//! The split view's secondary pane — a read-only transcript rendered next
//! to the active chat at half width. It shares the main pane's look (same
//! cards, Markdown, day separators) but none of its affordances: no
//! composer, footer actions, context menu, collapse, undo, or approval
//! buttons — every chat op stays bound to the active pane. Card expand
//! toggles are the one exception: they flip view state on the *secondary*
//! chat and re-measure this pane's own scroller.
//!
//! Element ids and keyed state are `split-`-namespaced so a message index
//! can't collide with the same row in the main pane.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::text::CodeBlock;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageHeader};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{Chat, ChatMessage, MessageKind, Role};
use crate::views::cards::render_plan;
use crate::views::markdown::{MarkdownState, copy_code_button};
use crate::workspace::Workspace;

/// High bit OR-ed into per-block keys (code-copy flags, mermaid state) so
/// the split pane's keyed state never aliases the main pane's.
const SPLIT_KEY: u64 = 1 << 62;

impl Workspace {
    /// The chat column plus — while `secondary` is set — a divider and the
    /// read-only split pane, each taking half the row. The whole pane is a
    /// `ChatDrag` drop target: releasing a dragged sidebar chat here tears
    /// it off into its own window (`tear_off_hint` marks the affordance).
    pub fn render_chat_row(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ws = cx.entity();
        let ws_drop = cx.entity();
        div()
            .id("chat-pane")
            .test_support()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .on_drag_move::<super::sidebar_row::ChatDrag>(move |ev: &DragMoveEvent<super::sidebar_row::ChatDrag>, _, cx| {
                ws.update(cx, |this, cx| {
                    // Ephemeral chats never reach disk — another window
                    // can't load one, so the affordance doesn't apply to
                    // their drags.
                    let droppable = this.chats.iter().any(|c| c.id == ev.drag(cx).id && !c.ephemeral);
                    this.set_tear_off_hover(droppable && ev.bounds.contains(&ev.event.position), cx);
                });
            })
            .on_drop::<super::sidebar_row::ChatDrag>(move |drag, _, cx| {
                ws_drop.update(cx, |this, cx| {
                    this.set_tear_off_hover(false, cx);
                    this.open_chat_in_new_window(drag.id, cx);
                });
            })
            .child(self.render_chat(window, cx))
            .when_some(self.secondary, |d, six| {
                d.child(div().w(px(1.)).h_full().flex_shrink_0().bg(cx.theme().border))
                    .child(self.render_split_pane(six, window, cx))
            })
            .when(self.tear_off_hover && cx.has_active_drag(), |d| d.child(crate::chat_window::tear_off_hint(cx)))
    }

    /// The read-only pane for `chats[six]`: a slim titlebar (click swaps
    /// the panes, × closes the split) over the chat's own scroller.
    fn render_split_pane(&mut self, six: usize, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(chat) = self.chats.get(six) else {
            return div().flex_1().min_w_0();
        };
        let messages: Rc<Vec<ChatMessage>> = chat.messages.clone();
        let running = chat.running;
        let titlebar = split_titlebar(chat, cx.entity(), cx);
        // Keep the pane's scroller in step with the transcript: appends
        // grow it (preserving scroll position), a shorter transcript means
        // a truncation — reset. A running turn re-measures every row so
        // streaming text and tool output can't render clipped.
        let count = messages.len();
        self.secondary_scroller.update(cx, |s, cx| {
            let known = s.item_count();
            if count > known {
                s.append(count - known, cx);
            } else if count < known {
                s.reset(count, cx);
            }
            if running && count > 0 {
                s.remeasure_items(0..count, cx);
            }
        });
        let ws = cx.entity();
        let compact = self.compact_mode;
        let list = MessageScroller::new("split-messages", self.secondary_scroller.clone(), move |ix, window, cx| {
            let prev_at = ix.checked_sub(1).and_then(|p| messages.get(p)).map(|m| m.at);
            let at = messages.get(ix).map(|m| m.at);
            let row = messages
                .get(ix)
                .map(|msg| split_message(ix, msg, &ws, window, cx))
                .unwrap_or_else(|| div().into_any_element());
            crate::views::date_separator::separator_row(crate::views::date_separator::SeparatorRow { ix, at, prev_at, row, compact }, cx)
        });
        // Same density overrides as the main transcript's scroller.
        let list = crate::views::message::density_scroller(list, compact);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().background)
            .child(titlebar)
            .child(div().flex_1().min_h_0().child(list))
    }
}

/// The pane's top strip: window-drag surface like the main titlebar, plus
/// the chat's title and a close button. A plain click promotes the pane —
/// the chat becomes active and the old active takes the split slot.
fn split_titlebar(chat: &Chat, ws: Entity<Workspace>, cx: &mut App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let ws_close = ws.clone();
    crate::window::titlebar_drag(
        div()
            .id("split-titlebar")
            .flex()
            .items_center()
            .gap_2()
            .h(px(crate::window::TOP_BAR_H))
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .cursor_pointer()
            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(chat.title.clone()))
            .when_some(chat.color, |d, color| d.child(crate::views::chat_menu::color_dot(("split-color-dot", chat.id), color, px(8.))))
            .when(chat.worktree, |d| d.child(crate::views::chat_menu::worktree_badge("split-worktree-badge", &chat.workdir, cx)))
            .when(chat.ephemeral, |d| d.child(crate::views::chat_menu::temp_badge("split-temp-badge", cx)))
            .child(div().flex_1())
            .child(
                div()
                    .id("split-close")
                    .test_support()
                    .p_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(cx.theme().muted_foreground)
                    .child(IconName::X)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        ws_close.update(cx, |this, cx| this.close_split(cx));
                    }),
            )
            .on_click(move |_, window, cx| {
                ws.update(cx, |this, cx| this.activate_secondary(window, cx));
            }),
    )
    .test_support()
}

/// One transcript row in the split pane — the read-only counterpart of
/// `render_message`: same card shapes, no editing affordances.
fn split_message(ix: usize, msg: &ChatMessage, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    match &msg.kind {
        MessageKind::Text(_) => split_text(ix, msg, ws, window, cx),
        MessageKind::Tool(tool) => super::split_cards::split_tool(ix, tool, ws, cx).into_any_element(),
        MessageKind::Diff(diff) => super::split_cards::split_diff(ix, diff, ws, cx).into_any_element(),
        MessageKind::Plan(plan) => render_plan(ix, plan, cx).into_any_element(),
        MessageKind::Approval(card) => super::split_cards::split_approval(ix, card, cx).into_any_element(),
    }
}

/// A text bubble without the main pane's chrome: no collapse clip, inline
/// editor, undo button, context menu, or footer — just the body (Markdown
/// for assistant replies) and image thumbnails.
fn split_text(ix: usize, msg: &ChatMessage, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let (word_wrap, font_size, compact) = {
        let ws = ws.read(cx);
        (ws.word_wrap, ws.font_size, ws.compact_mode)
    };
    let d = crate::views::message::density(compact);
    let alignment = match msg.role {
        Role::User => MessageAlignment::End,
        Role::Assistant => MessageAlignment::Start,
    };
    let md_state = (msg.role == Role::Assistant).then(|| split_markdown_state(ix, text, window, cx));
    let thumbs: Vec<AnyElement> = msg
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, a)| crate::attachment::is_image_path(a))
        .map(|(j, a)| {
            let ws_thumb = ws.clone();
            let path = a.to_string();
            div()
                .id(SharedString::from(format!("split-msg-thumb-{ix}-{j}")))
                .cursor_pointer()
                .child(
                    img(std::path::PathBuf::from(&path))
                        .size(px(96.))
                        .rounded_md()
                        .object_fit(ObjectFit::Cover)
                        .with_fallback(|| IconName::Image.into_any_element()),
                )
                .on_click(move |_, _, cx| {
                    ws_thumb.update(cx, |this, cx| this.open_image_view(path.clone(), cx));
                })
                .into_any_element()
        })
        .collect();
    let body = div()
        .id(("split-md-body", ix))
        .px_4()
        .py(d.body_py)
        .text_size(px(font_size))
        .when(msg.role == Role::User, |d| d.rounded_lg().bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
        .when(!thumbs.is_empty(), |d| d.child(div().flex().flex_wrap().gap_2().pb_1().children(thumbs)))
        .child(if let Some(state) = &md_state {
            split_markdown(ix, text, state, ws, cx)
        } else {
            div()
                .whitespace_nowrap()
                .when(word_wrap, |d| d.whitespace_normal())
                .child(text.clone())
                .into_any_element()
        });
    let mut message = Message::new().alignment(alignment).content(MessageContent::new().child(body));
    if msg.role == Role::Assistant {
        message = message.header(
            MessageHeader::new().child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(IconName::Bot)
                    .child("Rixl"),
            ),
        );
    }
    div().id(("split-msg", ix)).child(message).into_any_element()
}

/// The pane's own keyed `MarkdownState` — a `split`-prefixed key keeps it
/// independent of the main pane's state for the same message index.
fn split_markdown_state(ix: usize, text: &str, window: &mut Window, cx: &mut App) -> Entity<MarkdownState> {
    window.use_keyed_state(("md-split", ix), cx, |_, cx| MarkdownState::new(text, cx))
}

/// Assistant Markdown for the split pane: rendered blocks plus mermaid,
/// but code blocks carry only the copy button — Run/Apply would act on
/// the active chat, so a read-only pane doesn't offer them.
fn split_markdown(ix: usize, text: &SharedString, state: &Entity<MarkdownState>, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    state.update(cx, |state, cx| state.sync(text, cx));
    let compact = ws.read(cx).compact_mode;
    let ws = ws.clone();
    TextView::new(&state.read(cx).view)
        .style(super::message::markdown_style(compact))
        .code_block_actions(move |block, window, cx| split_code_actions(ix, block, window, cx))
        .markdown_block_parser(super::mermaid::parse_block)
        .markdown_block_renderer("mermaid", move |node, window, cx| super::mermaid::render_block(ix, node, window, cx))
        .on_link_click(move |url, event, _, cx| super::markdown::open_link(url, event, &ws, cx))
        .into_any_element()
}

/// Read-only code-block affordances: the language tag and a copy button
/// keyed into the split namespace so its copied flag is the pane's own.
fn split_code_actions(ix: usize, block: &CodeBlock, window: &mut Window, cx: &mut App) -> AnyElement {
    let key = block.span.as_ref().map(|s| s.start).unwrap_or(0) as u64 | SPLIT_KEY;
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .when_some(block.lang(), |d, lang| d.child(div().id(ElementId::Name(format!("split-code-lang-{ix}-{lang}").into())).child(lang)))
        .child(copy_code_button(ix, key, block.code().to_string(), window, cx))
        .into_any_element()
}
