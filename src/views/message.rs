use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{MessageKind, Role, ToolCall};

use crate::views::approval::render_approval;
use crate::views::cards::{MsgCtx, render_diff, render_plan, render_tool_call, render_tool_group, tool_group};
use crate::workspace::Workspace;
/// Transcript spacing for one density. `density(compact)` maps
/// `Workspace::compact_mode` onto the spacing values the message row, its
/// footer, the day separator and the scroller's row gap read — the defaults
/// below match the stock spacing (`py_2`, `pb_8`, `rems(0.625)` gaps) so off
/// is a no-op.
#[derive(Clone, Copy)]
pub(crate) struct Density {
    /// Vertical padding inside the message bubble (`md-body`).
    pub body_py: Pixels,
    /// Gap between a message's header/content/footer slots.
    pub stack_gap: Rems,
    /// Vertical padding around a day-separator label.
    pub separator_py: Pixels,
    /// Bottom padding the scroller puts between rows.
    pub row_gap: Pixels,
    /// Padding inside a fenced code block.
    pub code_block_p: Pixels,
    /// Vertical gap between Markdown blocks (paragraphs, lists, code).
    pub paragraph_gap: Rems,
}

/// The spacing set for `Workspace::compact_mode` — compact trades the
/// transcript's airy rhythm for denser rows.
pub(crate) fn density(compact: bool) -> Density {
    if compact {
        Density {
            body_py: px(4.),
            stack_gap: rems(0.25),
            separator_py: px(2.),
            row_gap: px(8.),
            code_block_p: px(6.),
            paragraph_gap: rems(0.5),
        }
    } else {
        Density {
            body_py: px(8.),
            stack_gap: rems(0.625),
            separator_py: px(8.),
            row_gap: px(32.),
            code_block_p: px(12.),
            paragraph_gap: rems(1.),
        }
    }
}

/// The `TextViewStyle` the transcript's Markdown renders with — the
/// component style folds onto the themed one, so setting the density's
/// paragraph gap and code-block padding leaves every other field themed.
pub(crate) fn markdown_style(compact: bool) -> gpui_kit::component::text::TextViewStyle {
    let d = density(compact);
    gpui_kit::component::text::TextViewStyle {
        paragraph_gap: d.paragraph_gap,
        code_block: StyleRefinement::default().p(d.code_block_p),
        ..Default::default()
    }
}

/// Apply compact row spacing to a transcript `MessageScroller`: the row
/// style overrides the scroller's stock `pb_8` between rows; the list style
/// keeps the last row's bottom edge at the same inset (its `pb` would
/// otherwise stack on top). Off is a no-op — the stock styles stay.
pub(crate) fn density_scroller(
    s: gpui_kit::component::message_scroller::MessageScroller, compact: bool,
) -> gpui_kit::component::message_scroller::MessageScroller {
    if !compact {
        return s;
    }
    let d = density(true);
    s.with_row_style(StyleRefinement::default().pb(d.row_gap))
        .with_list_style(StyleRefinement::default().pt(d.body_py).pb(px(0.)))
}

pub fn render_message(mc: MsgCtx, focused: bool, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    let el = match &msg.kind {
        MessageKind::Text(_) => render_text(mc, ws, window, cx),
        MessageKind::Tool(tool) => render_tool(mc, tool, ws, cx),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
        MessageKind::Plan(plan) => render_plan(ix, plan, cx).into_any_element(),
        MessageKind::Approval(card) => render_approval(ix, card, ws.clone(), cx).into_any_element(),
    };
    crate::msg_nav::wrap_nav_focus(el, focused, cx)
}

/// A tool message renders one of three ways: a lone call keeps its plain
/// card; the head of a 2+ run shows the collapsible "N tool calls" summary
/// (embedding its own call when open); later members render indented cards
/// only while the group is expanded — collapsed they leave an empty row.
/// Grouping is skipped under a chat-search filter so every match stays
/// visible.
fn render_tool(mc: MsgCtx, tool: &ToolCall, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    enum Row {
        Single,
        Head(crate::views::cards::ToolGroup, bool, std::rc::Rc<Vec<crate::model::ChatMessage>>),
        Member(bool),
    }
    let row = {
        let ws = ws.read(cx);
        let chat = &ws.chats[ws.active];
        let filtered = ws.chat_search_open && !ws.chat_search.read(cx).value().is_empty();
        match (!filtered).then(|| tool_group(&chat.messages, ix)).flatten() {
            None => Row::Single,
            Some(g) if g.head == ix => {
                let expanded = chat.expanded_tool_groups.contains(&(g.head, msg.at));
                Row::Head(g, expanded, chat.messages.clone())
            },
            Some(g) => {
                let expanded = chat.messages.get(g.head).is_some_and(|h| chat.expanded_tool_groups.contains(&(g.head, h.at)));
                Row::Member(expanded)
            },
        }
    };
    match row {
        Row::Single => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        Row::Head(g, expanded, messages) => render_tool_group(g, &messages, expanded, ws.clone(), cx).into_any_element(),
        Row::Member(true) => div().pl_4().child(render_tool_call(ix, tool, ws.clone(), cx)).into_any_element(),
        Row::Member(false) => div().into_any_element(),
    }
}

fn render_text(mc: MsgCtx, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, msg, .. } = mc;
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let role = msg.role;
    let (word_wrap, font_size, compact, undoable, edit_input, running, last_msg) = {
        let ws = ws.read(cx);
        let chat = &ws.chats[ws.active];
        let input = ws
            .editing
            .as_ref()
            .filter(|e| e.chat_id == chat.id && e.ix == ix && e.at == msg.at)
            .map(|e| e.input.clone());
        // "Undo turn" lives on the last user message only — it rewinds the
        // transcript to that point, so an older message would drop later
        // turns too (the Snapshots panel restores files without truncating).
        let last_user = chat.messages.iter().rposition(|m| m.role == Role::User) == Some(ix);
        (
            ws.word_wrap,
            ws.font_size,
            ws.compact_mode,
            input.is_none() && !chat.running && last_user && crate::checkpoints::for_message(chat, ix).is_some(),
            input,
            chat.running,
            ix + 1 == chat.messages.len(),
        )
    };
    let d = density(compact);
    // Long messages clip behind a fade + "Show more" bar — never the
    // streaming tail (it classifies once `running` clears) or the editor.
    let collapse = crate::views::chat_collapse::collapse_state(ws.read(cx), ix, msg, edit_input.is_some());
    let alignment = match role {
        Role::User => MessageAlignment::End,
        Role::Assistant => MessageAlignment::Start,
    };
    // Assistant bodies carry a keyed MarkdownState — the footer reads it for
    // the view-raw toggle, so it is created here and shared.
    let md_state = if role == Role::Assistant {
        Some(super::markdown::markdown_state(ix, text, window, cx))
    } else {
        None
    };
    // While the message is being edited the bubble becomes an inline
    // editor — Enter resends, Esc cancels (see `views::message_edit`).
    let body = if let Some(input) = edit_input {
        super::message_edit::message_editor(ix, &input, ws, cx)
    } else {
        // Image attachments render as thumbnails inside the bubble — a click
        // opens the lightbox (see `crate::image_view`).
        let thumbs: Vec<AnyElement> = msg
            .attachments
            .iter()
            .enumerate()
            .filter(|(_, a)| crate::attachment::is_image_path(a))
            .map(|(j, a)| {
                let ws_thumb = ws.clone();
                let path = a.to_string();
                div()
                    .id(SharedString::from(format!("msg-thumb-{ix}-{j}")))
                    .test_support()
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
        div()
            .id(("md-body", ix))
            .test_support()
            .px_4()
            .py(d.body_py)
            .text_size(px(font_size))
            // Codex: user text sits in a tinted bubble; assistant replies are
            // flat Markdown on the chat surface — no bubble.
            .when(role == Role::User, |d| {
                d.rounded_lg().bg(cx.theme().accent).text_color(cx.theme().accent_foreground)
            })
            .when(!thumbs.is_empty(), |d| {
                d.child(div().flex().flex_wrap().gap_2().pb_1().children(thumbs))
            })
            .child(if let Some(state) = &md_state {
                super::markdown::assistant_markdown(ix, text, state, ws, cx)
            } else {
                div()
                    .whitespace_nowrap()
                    .when(word_wrap, |d| d.whitespace_normal())
                    .child(text.clone())
                    .into_any_element()
            })
            .into_any_element()
    };
    let body = crate::views::chat_collapse::collapse_wrap(mc, body, collapse, ws, cx);

    let mut message = Message::new()
        .alignment(alignment)
        .content(MessageContent::new().child(body))
        .with_stack_style(StyleRefinement::default().gap(d.stack_gap))
        .gap(d.stack_gap);
    // Group for hover-revealed footer actions.
    if role == Role::Assistant {
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
    message = message.footer(MessageFooter::new().child(super::message_footer::message_footer(mc, ws, md_state.clone(), cx)));
    let ws_menu = ws.clone();
    let ws_revert = ws.clone();
    let source = text.clone();
    let group = SharedString::from(format!("msg-{ix}"));
    div()
        .id(("msg", ix))
        .test_support()
        .group(group.clone())
        .child(message)
        // "Undo turn" on the last user message — hover-revealed like the
        // footer actions, hidden while a reply runs.
        .when(undoable, |d| {
            d.child(
                div().flex().justify_end().child(
                    div()
                        .id(("undo", ix))
                        .test_support()
                        .flex()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::Undo2)
                        .child("Undo turn")
                        .on_click(move |_, window, cx| {
                            ws_revert.update(cx, |this, cx| this.undo_turn(ix, window, cx));
                        }),
                ),
            )
        })
        .context_menu(super::message_menu::msg_menu(&mc, ws_menu, md_state, source, super::message_menu::MenuGates { undoable, running, last_msg }))
        .into_any_element()
}
