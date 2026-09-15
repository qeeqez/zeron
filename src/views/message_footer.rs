//! The message row's hover-revealed footer: ghost action icons (copy,
//! edit, view-raw, retry, rating, read-aloud) plus the turn duration,
//! token usage and timestamp.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Role;
use crate::views::cards::MsgCtx;
use crate::views::markdown::MarkdownState;
use crate::workspace::Workspace;

/// Flip message `ix` between rendered Markdown and its source. The flag
/// lives on the keyed `MarkdownState`; the row's height changes, so the
/// virtual scroller re-measures it (filtered position, like expand).
pub(super) fn toggle_raw(state: Entity<MarkdownState>, ws: Entity<Workspace>, ix: usize) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        state.update(cx, |md, cx| {
            md.raw = !md.raw;
            cx.notify();
        });
        ws.update(cx, |this, cx| {
            let pos = this.filtered_pos(ix, cx);
            this.scroller.update(cx, |s, cx| s.remeasure_items(pos..pos + 1, cx));
        });
    }
}

/// One ghost icon in the hover toolbar — invisible until the `msg-{ix}`
/// group is hovered.
fn action_icon(
    id: (&'static str, usize), icon: IconName, color: Hsla, group: &SharedString,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .cursor_pointer()
        .invisible()
        .group_hover(group.clone(), |style| style.visible())
        .text_color(color)
        .child(icon)
        .on_click(on_click)
}

/// Hover-revealed action row under a message: copy, edit (user messages),
/// view-raw, retry (last assistant reply only), rating, read-aloud, then
/// the turn duration, token usage and timestamp.
pub(super) fn message_footer(mc: MsgCtx, ws: &Entity<Workspace>, md_state: Option<Entity<MarkdownState>>, cx: &mut App) -> Div {
    let MsgCtx { ix, is_last, msg, .. } = mc;
    let muted = hsla(0.0, 0.0, 0.55, 1.0);
    let accent = cx.theme().accent;
    let group = SharedString::from(format!("msg-{ix}"));
    let mut row = div().flex().items_center().gap_1();
    {
        let ws = ws.clone();
        row = row.child(action_icon(("copy", ix), IconName::Copy, muted, &group, move |_, _, cx| {
            ws.update(cx, |this, cx| this.copy_message(ix, cx));
        }));
    }
    if msg.role == Role::User {
        // Edit reopens the message inline — commit truncates after it and
        // resends the edited text (see `Workspace::commit_edit`).
        let ws = ws.clone();
        row = row.child(action_icon(("edit", ix), IconName::Pencil, muted, &group, move |_, window, cx| {
            ws.update(cx, |this, cx| this.edit_message(ix, window, cx));
        }));
    }
    if msg.role == Role::Assistant {
        // View-raw flips the body between rendered Markdown and source.
        if let Some(md) = md_state {
            let color = if md.read(cx).raw { accent } else { muted };
            row = row.child(action_icon(("raw", ix), IconName::Code, color, &group, toggle_raw(md, ws.clone(), ix)));
        }
        // retry_last re-runs the final turn — only meaningful on the last
        // message, so the icon is gated to it.
        if is_last {
            let ws = ws.clone();
            row = row.child(action_icon(("retry", ix), IconName::RotateCcw, muted, &group, move |_, _, cx| {
                ws.update(cx, |this, cx| this.retry_last(cx));
            }));
        }
        let rating = msg.rating;
        for (id, icon, up) in [("up", IconName::ThumbsUp, true), ("down", IconName::ThumbsDown, false)] {
            let ws = ws.clone();
            let color = if rating == Some(up) { accent } else { muted };
            row = row.child(action_icon((id, ix), icon, color, &group, move |_, _, cx| {
                ws.update(cx, |this, cx| this.rate_message(ix, up, cx));
            }));
        }
        {
            let ws = ws.clone();
            row = row.child(action_icon(("speak", ix), IconName::Volume2, muted, &group, move |_, _, cx| {
                ws.update(cx, |this, _cx| this.speak_message(ix));
            }));
        }
        if let Some(dur) = mc.duration {
            row = row.child(
                div()
                    .id(("worked", ix))
                    .test_support()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("Worked for {}s", dur.as_secs())),
            );
        }
    }
    row.child(div().flex_1())
        .when_some(msg.usage, |d, u| d.child(div().text_xs().text_color(muted).child(format!("{} in · {} out", u.input, u.output))))
        .child(div().text_xs().text_color(muted).child(format_time(msg.at)))
}

fn format_time(at: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at).format("%H:%M").to_string()
}
