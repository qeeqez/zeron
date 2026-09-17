//! The message row's hover-revealed footer: ghost action icons (copy,
//! quote, edit, view-raw, regenerate, rating, read-aloud) plus the turn
//! duration, token usage and timestamp. A thumbs-down also mounts a "what
//! went wrong" note editor under the row (see `crate::feedback`).

mod pager;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};

use gpui_kit::component::input::{Escape as InputEscape, Input};
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

/// Hover-revealed action row under a message: copy, quote, edit (user
/// messages), view-raw, regenerate (assistant replies), rating,
/// read-aloud, then the turn duration, token usage and timestamp.
pub(super) fn message_footer(mc: MsgCtx, ws: &Entity<Workspace>, md_state: Option<Entity<MarkdownState>>, cx: &mut App) -> Div {
    let MsgCtx { ix, msg, .. } = mc;
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
    {
        // Quote seeds the composer with the message as a `>` reply block.
        let ws = ws.clone();
        row = row.child(action_icon(("quote", ix), IconName::Quote, muted, &group, move |_, window, cx| {
            ws.update(cx, |this, cx| this.quote_message(ix, window, cx));
        }));
    }
    {
        // Bookmark stars the message for the chat ⋯ menu's Bookmarks list.
        // A starred row keeps the icon pinned (filled, accent) so the state
        // reads without hovering; unstarred it hides with the other ghosts.
        let ws = ws.clone();
        let bookmarked = msg.bookmarked;
        let icon = action_icon(
            ("bookmark", ix),
            if bookmarked { IconName::StarFill } else { IconName::Star },
            if bookmarked { accent } else { muted },
            &group,
            move |_, _, cx| {
                ws.update(cx, |this, cx| this.toggle_bookmark(ix, cx));
            },
        );
        row = row.child(icon.when(bookmarked, |el| el.visible()));
    }
    {
        // Pin marks the message for the banner under the titlebar — same
        // hover-ghost/pinned-visible treatment as the bookmark star.
        let ws = ws.clone();
        let pinned = msg.pinned;
        let icon = action_icon(("pin", ix), IconName::Pin, if pinned { accent } else { muted }, &group, move |_, _, cx| {
            ws.update(cx, |this, cx| this.toggle_message_pin(ix, cx));
        });
        row = row.child(icon.when(pinned, |el| el.visible()));
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
        // A regenerated reply keeps its predecessors — the pager swaps
        // them back in (see `Workspace::cycle_alternative`).
        if !msg.alternatives.is_empty() {
            row = row.child(pager::version_pager(mc, ws));
        }
        // View-raw flips the body between rendered Markdown and source.
        if let Some(md) = md_state {
            let color = if md.read(cx).raw { accent } else { muted };
            row = row.child(action_icon(("raw", ix), IconName::Code, color, &group, toggle_raw(md, ws.clone(), ix)));
        }
        // A failed turn's row gets a real Retry button — always visible,
        // disabled while a turn runs — instead of the ghost icon. It
        // re-sends the turn's prompt through `regenerate_from`, which
        // parks the error in the new reply's alternatives.
        if msg.is_error() {
            let running = ws.read(cx).chats[ws.read(cx).active].running;
            let ws_retry = ws.clone();
            row = row.child(
                Button::new(("retry", ix))
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCcw)
                    .label("Retry")
                    .disabled(running)
                    .on_click(move |_, window, cx| {
                        ws_retry.update(cx, |this, cx| this.regenerate_from(ix, window, cx));
                    }),
            );
        } else {
            // Regenerate re-runs the turn that produced this reply — on the
            // last message it's a plain retry, mid-chat it truncates first.
            let ws = ws.clone();
            row = row.child(action_icon(("retry", ix), IconName::RotateCcw, muted, &group, move |_, window, cx| {
                ws.update(cx, |this, cx| this.regenerate_from(ix, window, cx));
            }));
        }
        let rating = msg.rating;
        for (id, icon, up) in [("up", IconName::ThumbsUp, true), ("down", IconName::ThumbsDown, false)] {
            let ws = ws.clone();
            let color = if rating == Some(up) { accent } else { muted };
            row = row.child(action_icon((id, ix), icon, color, &group, move |_, window, cx| {
                ws.update(cx, |this, cx| this.rate_message(ix, up, window, cx));
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
    let row = row
        .child(div().flex_1())
        .when_some(msg.usage, |d, u| {
            // The chat's current model prices the estimate — turns predate
            // model switches are approximate, and unknown models show
            // tokens only.
            let model = {
                let ws = ws.read(cx);
                ws.chats[ws.active].model.clone()
            };
            let mut text = format!("{} in · {} out", u.input, u.output);
            if let Some(cost) = crate::pricing::model_pricing(&model)
                .map(|p| p.cost(crate::usage::TurnUsage { input: u.input, output: u.output, cached: 0 }))
            {
                text.push_str(&format!(" · ~{}", crate::pricing::fmt_cost(cost)));
            }
            d.child(div().text_xs().text_color(muted).child(text))
        })
        .child(div().text_xs().text_color(muted).child(format_time(msg.at)));
    let mut footer = div().flex().flex_col().child(row);
    if msg.role == Role::Assistant {
        let editing = ws.read(cx).feedback_editing(ix);
        if editing {
            footer = footer.child(feedback_editor(ix, ws, cx));
        } else if let Some(note) = Workspace::feedback_note(&ws.read(cx).chats[ws.read(cx).active], msg.at).map(str::to_string) {
            footer = footer.child(feedback_note_row(ix, note, ws, cx));
        }
    }
    footer
}

fn format_time(at: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at).format("%H:%M").to_string()
}

/// The "what went wrong" editor under a thumbs-down: the shared feedback
/// input, a Save and a Cancel. Enter commits via the input's `PressEnter`
/// subscription (see `FeedbackState::new`); Escape cancels here.
fn feedback_editor(ix: usize, ws: &Entity<Workspace>, cx: &mut App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let ws_save = ws.clone();
    let ws_cancel = ws.clone();
    let ws_esc = ws.clone();
    div()
        .id(("feedback-edit", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .on_action(move |_: &InputEscape, window, cx| {
            cx.stop_propagation();
            ws_esc.update(cx, |this, cx| this.cancel_feedback(window, cx));
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&ws.read(cx).feedback.input).id(("feedback-input", ix)).appearance(true).w_full()),
        )
        .child(
            div()
                .id(("feedback-save", ix))
                .test_support()
                .cursor_pointer()
                .text_color(cx.theme().accent)
                .text_xs()
                .child("Save")
                .on_click(move |_, window, cx| {
                    ws_save.update(cx, |this, cx| this.commit_feedback(window, cx));
                }),
        )
        .child(
            div()
                .id(("feedback-cancel", ix))
                .test_support()
                .cursor_pointer()
                .text_color(cx.theme().muted_foreground)
                .text_xs()
                .child("Cancel")
                .on_click(move |_, window, cx| {
                    ws_cancel.update(cx, |this, cx| this.cancel_feedback(window, cx));
                }),
        )
}

/// A saved "what went wrong" note under the footer — always visible (unlike
/// the ghost icons), click to reopen the editor.
fn feedback_note_row(ix: usize, note: String, ws: &Entity<Workspace>, cx: &mut App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let ws = ws.clone();
    div()
        .id(("feedback-note", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .py_1()
        .cursor_pointer()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(IconName::MessageCircle)
        .child(note)
        .on_click(move |_, window, cx| {
            ws.update(cx, |this, cx| this.edit_feedback_note(ix, window, cx));
        })
}
