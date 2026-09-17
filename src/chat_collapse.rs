//! Collapsible long messages: a text message past `COLLAPSE_LINES` rendered
//! lines starts clipped behind a fade and a "Show more" bar, expanding in
//! place on click ("Show less" re-collapses). The expanded set lives on
//! `Chat` — runtime-only, keyed by message index + timestamp like
//! `expanded_tool_groups` so a truncated/rewritten transcript can't reopen
//! a different message. A streaming last message never collapses mid-turn;
//! it classifies once `running` clears. Find, chat search and message
//! navigation auto-expand their target via `expand_msg` so a hit inside the
//! clipped region is visible when the scroller lands on it.
//!
//! Declared from `views/mod.rs` via `#[path]` — `main.rs` is at the SLOC
//! cap (same pattern as `date_separator`).

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::cards::MsgCtx;
use crate::workspace::Workspace;

/// Rendered-line estimate at which a message starts collapsed — long
/// assistant replies and pasted dumps wall-scroll the transcript without it.
pub(crate) const COLLAPSE_LINES: usize = 40;
/// Characters per rendered line used by the estimate — a proxy for the
/// transcript's wrap width; only consulted when word wrap is on.
const WRAP_COLS: usize = 100;
/// Line-height multiplier matching the transcript's text metrics — the clip
/// height is `COLLAPSE_LINES` of these.
const LINE_HEIGHT: f32 = 1.4;
/// Fade band covering the clipped tail of a collapsed message.
const FADE_HEIGHT: f32 = 64.;

/// Estimated rendered height of `text` in lines: source lines plus the
/// wraps a long line takes at `WRAP_COLS` when word wrap is on (nowrap
/// renders each source line as one row, however wide). Markdown blocks
/// (headings, code fences) skew the estimate — it only needs to separate
/// wall-scroll replies from ordinary ones.
pub(crate) fn rendered_lines(text: &str, word_wrap: bool) -> usize {
    text.lines()
        .map(|line| if word_wrap { line.chars().count().div_ceil(WRAP_COLS).max(1) } else { 1 })
        .sum()
}

/// Whether `msg` is a text message long enough to collapse. `streaming` is
/// the still-growing last message of a running turn — it stays expanded
/// until the turn completes.
pub(crate) fn collapsible(msg: &ChatMessage, word_wrap: bool, streaming: bool) -> bool {
    if streaming {
        return false;
    }
    match &msg.kind {
        MessageKind::Text(text) => rendered_lines(text, word_wrap) > COLLAPSE_LINES,
        _ => false,
    }
}

/// A message's collapse state for this render.
pub(crate) enum Collapse {
    /// Short, streaming or being edited — rendered as-is.
    Off,
    /// Clipped at the threshold behind a fade + "Show more" bar.
    Collapsed,
    /// Full body with a "Show less" bar.
    Expanded,
}

/// Classify message `ix` for this render — `editing` is the inline editor
/// swap, which never collapses.
pub(crate) fn collapse_state(ws: &Workspace, ix: usize, msg: &ChatMessage, editing: bool) -> Collapse {
    let chat = &ws.chats[ws.active];
    let streaming = chat.running && ix + 1 == chat.messages.len();
    if editing || !collapsible(msg, ws.word_wrap, streaming) {
        return Collapse::Off;
    }
    if chat.expanded_msgs.contains(&(ix, msg.at)) { Collapse::Expanded } else { Collapse::Collapsed }
}

impl Workspace {
    /// The "Show more" / "Show less" bar's click: flip the message's
    /// expanded state and re-measure its scroller row.
    pub(crate) fn toggle_msg_collapse(&mut self, ix: usize, at: std::time::SystemTime, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        if !chat.expanded_msgs.remove(&(ix, at)) {
            chat.expanded_msgs.insert((ix, at));
        }
        self.remeasure_row(ix, cx);
        cx.notify();
    }

    /// Expand message `ix` when it is collapsed — called by the find/search/
    /// navigation jumps so a match inside the clipped region is visible on
    /// landing. No-op for short messages and the streaming tail.
    pub(crate) fn expand_msg(&mut self, ix: usize, cx: &mut Context<Self>) {
        let word_wrap = self.word_wrap;
        let chat = &mut self.chats[self.active];
        let streaming = chat.running && ix == chat.messages.len().saturating_sub(1);
        let Some(msg) = chat.messages.get(ix) else { return };
        if !collapsible(msg, word_wrap, streaming) || chat.expanded_msgs.contains(&(ix, msg.at)) {
            return;
        }
        let at = msg.at;
        chat.expanded_msgs.insert((ix, at));
        self.remeasure_row(ix, cx);
    }
}

/// The "Show more" / "Show less" bar under a collapsible message's body.
fn collapse_bar(mc: MsgCtx, label: &'static str, icon: IconName, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    let ws = ws.clone();
    let at = mc.msg.at;
    div()
        .id(("msg-collapse", mc.ix))
        .test_support()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .py_1()
        .cursor_pointer()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .hover(|d| d.text_color(cx.theme().foreground))
        .child(icon)
        .child(label)
        .on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| this.toggle_msg_collapse(mc.ix, at, cx));
        })
        .into_any_element()
}

/// Wrap a text message's body per its `Collapse` state: `Off` renders the
/// body unchanged, `Expanded` adds a "Show less" bar, `Collapsed` clips the
/// body at `COLLAPSE_LINES` worth of pixels behind a fade into the surface
/// (the user bubble's accent for user messages) plus a "Show more" bar.
pub(crate) fn collapse_wrap(mc: MsgCtx, body: AnyElement, state: Collapse, ws: &Entity<Workspace>, cx: &mut App) -> AnyElement {
    match state {
        Collapse::Off => body,
        Collapse::Expanded => div()
            .child(body)
            .child(collapse_bar(mc, "Show less", IconName::ChevronUp, ws, cx))
            .into_any_element(),
        Collapse::Collapsed => {
            let clip = px(ws.read(cx).font_size * LINE_HEIGHT * COLLAPSE_LINES as f32);
            // The fade ends on the color behind the text: the accent bubble
            // for a user message, the chat surface for an assistant reply.
            let surface = if mc.msg.role == Role::User { cx.theme().accent } else { cx.theme().background };
            div()
                .child(div().relative().child(div().h(clip).overflow_hidden().child(body)).child(
                    div().absolute().left_0().right_0().bottom_0().h(px(FADE_HEIGHT)).bg(linear_gradient(
                        180.,
                        linear_color_stop(surface.opacity(0.), 0.),
                        linear_color_stop(surface, 1.),
                    )),
                ))
                .child(collapse_bar(mc, "Show more", IconName::ChevronDown, ws, cx))
                .into_any_element()
        },
    }
}
