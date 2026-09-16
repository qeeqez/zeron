use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::send_queue::Queued;
use crate::workspace::Workspace;
/// Compact context/token meter for the composer footer: `+{turn} ·
/// {used} / {limit}` plus a subtle fill bar when the backend reports a
/// window size, else just the cumulative token count. `None` until the
/// first usage report — an untouched thread shows nothing.
pub fn usage_indicator(usage: &crate::usage::ChatUsage, cx: &App) -> Option<impl IntoElement> {
    let label = usage.label()?;
    let fill = usage.fill();
    Some(
        div()
            .id("usage-meter")
            .test_support()
            .aria_label(label.clone())
            .flex()
            .items_center()
            .gap_1p5()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when_some(fill, |d, fill| {
                d.child(
                    div()
                        .w(px(48.))
                        .h(px(4.))
                        .rounded_sm()
                        .bg(cx.theme().border)
                        .child(div().h_full().rounded_sm().w(px(48. * fill)).bg(cx.theme().muted_foreground)),
                )
            })
            .child(label),
    )
}

pub fn mention_item(file: &str, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let path = file.to_string();
    div()
        .id(SharedString::from(format!("mention-{file}")))
        .test_support()
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(format!("@{file}"))
        .on_click(move |_, window, cx| {
            apply_mention(&ws, &path, window, cx);
        })
}

pub fn attachment_chips(chat: &Chat, ws: &Entity<Workspace>, cx: &mut App) -> Vec<AnyElement> {
    chat.attachments
        .iter()
        .enumerate()
        .map(|(ix, path)| {
            let ws = ws.clone();
            let full = path.to_string();
            let name = std::path::Path::new(&full)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(full.clone());
            // Image attachments preview as a thumbnail; other files keep
            // the text icon. The marker id lets headless tests see the kind.
            // Clicking the thumbnail opens the lightbox; the name still
            // reveals the file and ✕ removes it.
            let preview: AnyElement = if crate::attachment::is_image_path(&full) {
                let ws_thumb = ws.clone();
                let thumb_path = full.clone();
                div()
                    .id(SharedString::from(format!("attach-thumb-{ix}")))
                    .test_support()
                    .cursor_pointer()
                    .child(
                        img(std::path::PathBuf::from(&full))
                            .size(px(20.))
                            .rounded_sm()
                            .object_fit(ObjectFit::Cover)
                            .with_fallback(|| IconName::Image.into_any_element()),
                    )
                    .on_click(move |_, _, cx| {
                        ws_thumb.update(cx, |this, cx| this.open_image_view(thumb_path.clone(), cx));
                    })
                    .into_any_element()
            } else {
                IconName::FileText.into_any_element()
            };
            div()
                .id(ix)
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(cx.theme().secondary)
                .text_xs()
                .child(preview)
                .child(
                    div()
                        .id(("reveal-attach", ix))
                        .cursor_pointer()
                        .child(name)
                        .on_click(move |_, _, cx| cx.reveal_path(std::path::Path::new(&full))),
                )
                .child(
                    div()
                        .id(("remove-attach", ix))
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::X)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.remove_attachment(ix, cx));
                        }),
                )
                .into_any_element()
        })
        .collect()
}

fn apply_mention(ws: &Entity<Workspace>, path: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.composer.update(cx, |s, cx| {
            let cur = s.value().to_string();
            let before = cur.rsplit_once('@').map(|(b, _)| b).unwrap_or("");
            s.set_value(format!("{before}@{path} "), window, cx);
            s.focus(window, cx);
        });
        // `set_value` suppresses Change — nudge the workspace so the menu closes.
        cx.notify();
    });
}

pub fn apply_pick(ws: &Entity<Workspace>, set: fn(&mut Workspace, &'static str), opt: &'static str, cx: &mut App) {
    ws.update(cx, |this, cx| {
        set(this, opt);
        cx.notify();
    });
}

/// A queued row's icon button — `id` is the test/click target, `icon` the
/// glyph, `disabled` greys out edge moves, `on_click` runs the queue op.
fn queue_button(
    id: SharedString, icon: IconName, disabled: bool, on_click: impl Fn(&mut Workspace, &mut Context<Workspace>) + 'static,
    ws: &Entity<Workspace>,
) -> impl IntoElement {
    let ws = ws.clone();
    div().id(id.clone()).test_support().child(
        Button::new(SharedString::from(format!("{id}-btn")))
            .ghost()
            .xsmall()
            .disabled(disabled)
            .icon(icon)
            .on_click(move |_, _, cx| ws.update(cx, |this, cx| on_click(this, cx))),
    )
}

/// One queued-message row: click the text to reopen it in the composer,
/// arrows reorder, the send icon jumps it to the front, ✕ drops it.
pub fn queued_item(item: &Queued, first: bool, last: bool, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let id = item.id;
    div()
        .id(SharedString::from(format!("queued-{id}")))
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .child(
            div()
                .id(SharedString::from(format!("queued-edit-{id}")))
                .test_support()
                .cursor_pointer()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.text.clone())
                .on_click({
                    let ws = ws.clone();
                    move |_, window, cx| {
                        ws.update(cx, |this, cx| this.edit_queued(id, window, cx));
                    }
                }),
        )
        .child(queue_button(
            SharedString::from(format!("queue-up-{id}")),
            IconName::ChevronUp,
            first,
            move |this, cx| this.move_queued(id, -1, cx),
            ws,
        ))
        .child(queue_button(
            SharedString::from(format!("queue-down-{id}")),
            IconName::ChevronDown,
            last,
            move |this, cx| this.move_queued(id, 1, cx),
            ws,
        ))
        .child(queue_button(
            SharedString::from(format!("queue-send-{id}")),
            IconName::Send,
            false,
            move |this, cx| this.send_queued_now(id, cx),
            ws,
        ))
        .child(queue_button(
            SharedString::from(format!("dequeue-{id}")),
            IconName::X,
            false,
            move |this, cx| this.remove_queued(id, cx),
            ws,
        ))
}
