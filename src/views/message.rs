use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader};
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::cards::{MsgCtx, message_footer, render_diff, render_tool_call};
use crate::workspace::Workspace;

pub fn render_message(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, .. } = mc;
    match &msg.kind {
        MessageKind::Text(_) => render_text(mc, msg, ws, cx),
        MessageKind::Tool(tool) => render_tool_call(ix, tool, ws.clone(), cx).into_any_element(),
        MessageKind::Diff(diff) => render_diff(ix, diff, ws.clone(), cx).into_any_element(),
    }
}

fn render_text(mc: MsgCtx, msg: &ChatMessage, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let MsgCtx { ix, .. } = mc;
    let MessageKind::Text(text) = &msg.kind else { unreachable!() };
    let role = msg.role;
    let word_wrap = ws.read(cx).word_wrap;
    let font_size = ws.read(cx).font_size;
    let alignment = match role {
        Role::User => MessageAlignment::End,
        Role::Assistant => MessageAlignment::Start,
    };
    let body = div()
        .px_4()
        .py_2()
        .rounded_lg()
        .text_size(px(f32::from(font_size)))
        .when(role == Role::User, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
        .when(role == Role::Assistant, |d| d.bg(cx.theme().secondary).text_color(cx.theme().foreground))
        .child(if role == Role::Assistant {
            TextView::markdown(("md", ix), text.clone())
                .selectable(true)
                .code_block_actions(|block, _window, _cx| {
                    let code = block.code().to_string();
                    div()
                        .id("copy-code")
                        .cursor_pointer()
                        .text_color(hsla(0.0, 0.0, 0.55, 1.0))
                        .child(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                        })
                })
                .into_any_element()
        } else {
            div()
                .whitespace_nowrap()
                .when(word_wrap, |d| d.whitespace_normal())
                .child(text.clone())
                .into_any_element()
        });

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
    message = message.footer(MessageFooter::new().child(message_footer(mc, msg, ws, cx)));
    let ws_menu = ws.clone();
    div()
        .id(("msg", ix))
        .test_support()
        .group(SharedString::from(format!("msg-{ix}")))
        .child(message)
        .context_menu(move |menu, _window, _cx| {
            let ws_copy = ws_menu.clone();
            let ws_retry = ws_menu.clone();
            let ws_edit = ws_menu.clone();
            let menu = menu.item(
                gpui_kit::component::menu::PopupMenuItem::new("Copy")
                    .icon(IconName::Copy)
                    .on_click(move |_, _, cx| {
                        ws_copy.update(cx, |this, cx| this.copy_message(ix, cx));
                    }),
            );
            let menu =
                if role == Role::User {
                    menu.item(gpui_kit::component::menu::PopupMenuItem::new("Edit").icon(IconName::Pencil).on_click(
                        move |_, window, cx| {
                            ws_edit.update(cx, |this, cx| this.edit_message(ix, window, cx));
                        },
                    ))
                } else {
                    menu
                };
            if role == Role::Assistant && mc.is_last {
                menu.item(
                    gpui_kit::component::menu::PopupMenuItem::new("Retry")
                        .icon(IconName::RotateCcw)
                        .on_click(move |_, _, cx| {
                            ws_retry.update(cx, |this, cx| this.retry_last(cx));
                        }),
                )
            } else {
                menu
            }
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use crate::model::Role;
    use crate::workspace::Workspace;

    /// Mount a `Workspace` in a headless window with `HOME` redirected to a
    /// temp dir so settings/chats stay off the real profile.
    fn mount(cx: &mut TestAppContext) -> (gpui_kit::Entity<Workspace>, &mut VisualTestContext) {
        let dir = std::env::temp_dir().join(format!("rixlcode-msg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
        cx.update(gpui_kit::init);
        cx.add_window_view(Workspace::new)
    }

    /// Seed the active chat with a completed assistant turn.
    fn seed_reply(ws: &gpui_kit::Entity<Workspace>, cx: &mut VisualTestContext) {
        ws.update(cx, |this, cx| {
            this.push_note("reply body".into(), cx);
            this.chats[this.active].last_turn = Some(std::time::Duration::from_secs(7));
        });
    }

    #[test]
    fn assistant_actions_reveal_on_hover() {
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        seed_reply(&ws, cx);
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            // Ghost icons exist but stay invisible until the row is hovered.
            assert!(!window.find(("copy", 0usize)).visible(), "copy hidden before hover");
            window.hover(("msg", 0usize), cx);
            window.draw(cx).clear(cx);
            for id in ["copy", "up", "down", "speak"] {
                assert!(window.find((id, 0usize)).visible(), "{id} should reveal on hover");
            }
        });
    }

    #[test]
    fn completed_turn_shows_duration_and_feedback_toggles() {
        let mut app = TestAppContext::single();
        let (ws, cx) = mount(&mut app);
        seed_reply(&ws, cx);
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(window.find(("worked", 0usize)).visible(), "duration label should show");
            window.hover(("msg", 0usize), cx);
            window.draw(cx).clear(cx);
            window.click(("up", 0usize), cx);
            assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(true));
            window.draw(cx).clear(cx);
            window.click(("down", 0usize), cx);
            assert_eq!(ws.read(cx).chats[0].messages[0].rating, Some(false));
        });
        // The seeded message is an assistant note — verify role for sanity.
        app.read(|cx| assert!(matches!(ws.read(cx).chats[0].messages[0].role, Role::Assistant)));
    }
}
