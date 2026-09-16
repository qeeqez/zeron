use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{MessageKind, Role, ToolCall};

use crate::views::approval::render_approval;
use crate::views::cards::{MsgCtx, render_diff, render_plan, render_tool_call, render_tool_group, tool_group};
use crate::workspace::Workspace;

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
    let (word_wrap, font_size, checkpointed, edit_input) = {
        let ws = ws.read(cx);
        let chat = &ws.chats[ws.active];
        let input = ws
            .editing
            .as_ref()
            .filter(|e| e.chat_id == chat.id && e.ix == ix && e.at == msg.at)
            .map(|e| e.input.clone());
        (ws.word_wrap, ws.font_size, input.is_none() && !chat.running && crate::checkpoints::for_message(chat, ix).is_some(), input)
    };
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
            .py_2()
            .text_size(px(f32::from(font_size)))
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

    let mut message = Message::new().alignment(alignment).content(MessageContent::new().child(body));
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
    // SharedString clones cheap — the menu closure scans it for fenced
    // blocks only when the menu actually opens.
    let source = text.clone();
    let bookmarked = msg.bookmarked;
    let group = SharedString::from(format!("msg-{ix}"));
    div()
        .id(("msg", ix))
        .test_support()
        .group(group.clone())
        .child(message)
        // "Undo turn" on the user message that opened a checkpointed turn —
        // hover-revealed like the footer actions, hidden while a reply runs.
        .when(checkpointed, |d| {
            d.child(
                div().flex().justify_end().child(
                    div()
                        .id(("revert", ix))
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
                        .on_click(move |_, _, cx| {
                            ws_revert.update(cx, |this, cx| this.revert_to_checkpoint(ix, cx));
                        }),
                ),
            )
        })
        .context_menu(move |menu, window, cx| {
            // Copy variants stay grouped at the top; Copy Code only appears
            // when the message actually has fenced blocks.
            let menu = menu
                .item(msg_item("Copy", IconName::Copy, &ws_menu, move |this, _w, cx| this.copy_message(ix, cx)))
                .item(msg_item("Copy as Markdown", IconName::FileCode, &ws_menu, move |this, _w, cx| {
                    this.copy_message_markdown(ix, cx)
                }))
                .when(!crate::chat_msg::copy::code_blocks(&source).is_empty(), |menu| {
                    menu.item(msg_item("Copy Code", IconName::SquareCode, &ws_menu, move |this, _w, cx| {
                        this.copy_message_code(ix, cx)
                    }))
                })
                .item(msg_item("Quote", IconName::Quote, &ws_menu, move |this, w, cx| this.quote_message(ix, w, cx)))
                // "Quote selection" appears only while this message's body has
                // an active selection — the text is captured as the menu opens
                // because the item's own click would clear it first.
                .when_some(md_state.as_ref().map(|md| md.read(cx).view.read(cx).selected_text()).filter(|s| !s.trim().is_empty()), |menu, selected| {
                    menu.item(msg_item("Quote selection", IconName::Quote, &ws_menu, move |this, w, cx| {
                        this.quote_selection(&selected, w, cx)
                    }))
                })
                .item(msg_item(if bookmarked { "Remove bookmark" } else { "Bookmark" }, IconName::Star, &ws_menu, move |this, _w, cx| {
                    this.toggle_bookmark(ix, cx)
                }))
                .separator()
                .item(msg_item("Fork here", IconName::GitFork, &ws_menu, move |this, w, cx| {
                    this.fork_chat(this.active, Some(ix), w, cx)
                }));
            // Splitting at the first message leaves nothing behind — the
            // item only exists where a prefix would remain.
            let menu = if ix > 0 {
                menu.item(msg_item("Split chat here", IconName::Scissors, &ws_menu, move |this, w, cx| {
                    let id = this.chats[this.active].id;
                    this.split_chat(id, ix, w, cx)
                }))
            } else {
                menu
            };
            let menu = if let Some(md) = md_state.clone() {
                let label = if md.read(cx).raw { "View rendered" } else { "View raw" };
                menu.item(PopupMenuItem::new(label).icon(IconName::Code).on_click(
                    super::message_footer::toggle_raw(md, ws_menu.clone(), ix),
                ))
            } else if checkpointed {
                menu.item(msg_item("Undo turn", IconName::Undo2, &ws_menu, move |this, _w, cx| {
                    this.revert_to_checkpoint(ix, cx)
                }))
            } else {
                menu
            };
            let menu = if role == Role::User {
                menu.item(msg_item("Edit", IconName::Pencil, &ws_menu, move |this, w, cx| this.edit_message(ix, w, cx)))
            } else {
                menu
            };
            match (role, mc.is_last) {
                (Role::Assistant, true) => super::retry_menu::retry_items(menu, &ws_menu, window, cx),
                (Role::Assistant, false) => menu.item(msg_item("Regenerate", IconName::RotateCcw, &ws_menu, move |this, w, cx| {
                    this.regenerate_from(ix, w, cx)
                })),
                _ => menu,
            }
        })
        .into_any_element()
}

/// One context-menu item that runs a `Workspace` method on click.
fn msg_item(
    label: &'static str, icon: IconName, ws: &Entity<Workspace>, f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> PopupMenuItem {
    let ws = ws.clone();
    PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
        ws.update(cx, |this, cx| f(this, window, cx));
    })
}
