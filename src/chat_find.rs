//! In-chat find (Cmd-F): a Codex-style find bar over the transcript. Unlike
//! `chat_search` — which filters the message list down to matches — find
//! keeps every message mounted, highlights the ones containing the query and
//! scrolls to the current match on Enter / Shift+Enter.
//!
//! A "match" is a message whose text contains the query (case-insensitive);
//! navigation and the `n / total` readout count matching messages, which is
//! also the granularity of the highlight.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::ChatMessage;
use crate::workspace::Workspace;

/// Indices of messages matching `query`, in transcript order. An empty query
/// matches nothing — the bar shows `0 / 0` rather than highlighting all.
pub(crate) fn matching_messages(messages: &[ChatMessage], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    (0..messages.len()).filter(|&ix| crate::chat_search::msg_matches(&messages[ix], &query)).collect()
}

/// Next match position after `cur`, wrapping both ways. `cur` may be stale
/// (the match list shrank since it was stored), so it is clamped first.
pub(crate) fn step_ix(cur: usize, back: bool, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let cur = cur.min(total - 1);
    if back { cur.checked_sub(1).unwrap_or(total - 1) } else { (cur + 1) % total }
}

/// Per-row find state handed to the message scroller: the sorted match list
/// plus the current match's message index (drives the stronger highlight).
pub(crate) struct FindMarks {
    matches: std::rc::Rc<Vec<usize>>,
    current: Option<usize>,
}

/// Wrap a rendered message in the find highlight when `real_ix` is a match.
/// Per-range highlighting isn't available in the markdown renderer, so the
/// whole message carries the mark — a translucent wash, stronger on the
/// current hit.
pub(crate) fn wrap_find_hit(el: AnyElement, real_ix: usize, find: Option<&FindMarks>, cx: &App) -> AnyElement {
    let Some(marks) = find else { return el };
    if marks.matches.binary_search(&real_ix).is_err() {
        return el;
    }
    let is_current = marks.current == Some(real_ix);
    let bg = if is_current { cx.theme().selection.alpha(0.6) } else { cx.theme().selection };
    div()
        .id(("find-hit", real_ix))
        .test_support()
        .aria_label(if is_current { "current find match" } else { "find match" })
        .w_full()
        .bg(bg)
        .child(el)
        .into_any_element()
}

/// Build the find input and wire its events: edits re-target the first match
/// (`find_query_changed`), Enter/Shift+Enter navigate (`find_jump`).
pub(crate) fn new_find_input(window: &mut Window, cx: &mut Context<Workspace>) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in chat"));
    cx.subscribe_in(&input, window, |this, _s, event: &InputEvent, _window, cx| match event {
        InputEvent::Change => this.find_query_changed(cx),
        InputEvent::PressEnter { shift, .. } => this.find_jump(*shift, cx),
        _ => {},
    })
    .detach();
    input
}

/// Find-bar state: the query input plus navigation bookkeeping. `last_query`
/// exists because Enter's propagated keystroke emits a spurious `Change` —
/// `find_query_changed` compares before resetting `match_ix`.
pub(crate) struct FindBar {
    pub input: Entity<InputState>,
    pub open: bool,
    /// Index of the current match within `find_matches` — drives the
    /// `n / total` readout and the stronger highlight on the current hit.
    pub match_ix: usize,
    pub last_query: String,
}

impl FindBar {
    pub(crate) fn new(input: Entity<InputState>) -> Self {
        Self { input, open: false, match_ix: 0, last_query: String::new() }
    }
}

impl Workspace {
    /// Matching message indices for the current find query — empty when the
    /// bar is closed or the query is blank.
    pub(crate) fn find_matches(&self, cx: &App) -> Vec<usize> {
        if !self.find.open {
            return Vec::new();
        }
        let query = self.find.input.read(cx).value().to_string();
        matching_messages(&self.chats[self.active].messages, &query)
    }

    /// Cmd-F / FindInChat: toggle the find bar. Opening focuses the input;
    /// closing clears the query and returns focus to the composer.
    pub fn open_chat_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = !self.find.open;
        self.find.match_ix = 0;
        if self.find.open {
            // The filter search and the find bar share the strip under the
            // titlebar — never show both.
            if self.chat_search_open {
                self.chat_search_open = false;
                self.chat_search.update(cx, |s, cx| s.set_value("", window, cx));
                let count = self.filtered_count(cx);
                self.scroller.update(cx, |s, cx| s.reset(count, cx));
            }
            let input = self.find.input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        } else {
            self.find.input.update(cx, |s, cx| s.set_value("", window, cx));
            self.find.last_query.clear();
            let composer = self.composer.clone();
            window.defer(cx, move |window, cx| {
                composer.update(cx, |s, cx| s.focus(window, cx));
            });
        }
        cx.notify();
    }

    /// Snapshot of find state for the scroller rows — the match list plus
    /// the current match's message index for the stronger highlight.
    pub(crate) fn find_marks(&self, cx: &App) -> FindMarks {
        let matches = std::rc::Rc::new(self.find_matches(cx));
        let current = matches.get(self.find.match_ix).copied();
        FindMarks { matches, current }
    }

    /// Enter in the find bar: jump to the next match; Shift+Enter: previous.
    /// `find_match_ix` indexes the match list; the scroller is addressed by
    pub fn find_jump(&mut self, back: bool, cx: &mut Context<Self>) {
        let matches = self.find_matches(cx);
        if matches.is_empty() {
            return;
        }
        self.find.match_ix = step_ix(self.find.match_ix, back, matches.len());
        self.scroll_to_message(matches[self.find.match_ix], cx);
        cx.notify();
    }
    /// A query edit re-targets the first match, like Codex's live find.
    /// Enter also emits `Change` (the propagated keystroke inserts a `\n`
    /// that single-line mode strips), so only a real text change resets.
    pub(crate) fn find_query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.find.input.read(cx).value().to_string();
        if query == self.find.last_query {
            return;
        }
        self.find.last_query = query;
        self.find.match_ix = 0;
        let matches = self.find_matches(cx);
        if let Some(&first) = matches.first() {
            self.scroll_to_message(first, cx);
        }
        cx.notify();
    }

    /// Open the find bar on `query` and land on message `msg_ix` — the
    /// global-search jump target. `last_query` is pre-seeded so the
    /// programmatic `set_value`'s Change event doesn't reset `match_ix`
    /// back to the first match (see `find_query_changed`).
    pub(crate) fn jump_to_message(&mut self, query: &str, msg_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.open {
            self.open_chat_find(window, cx);
        }
        self.find.last_query = query.to_string();
        self.find.input.update(cx, |s, cx| s.set_value(query, window, cx));
        let matches = self.find_matches(cx);
        self.find.match_ix = matches.iter().position(|&ix| ix == msg_ix).unwrap_or(0);
        if let Some(&target) = matches.get(self.find.match_ix) {
            self.scroll_to_message(target, cx);
        }
        cx.notify();
    }

    /// Esc while the find bar is open closes it (same path as the bar's ✕ —
    /// query cleared, focus back to the composer); otherwise the key falls
    /// through to the workspace handler (stop reply, close panels).
    pub(crate) fn find_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.open {
            self.open_chat_find(window, cx);
        } else {
            cx.propagate();
        }
    }

    /// The Cmd-F find bar: query input, `n / total` readout, prev/next and
    /// close. Enter/Shift+Enter in the input reach `find_jump` through the
    /// input's `PressEnter` event; Esc reaches `find_escape` via the chat
    /// column's `EscapeKey` listener.
    pub(crate) fn find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.find_matches(cx).len();
        let current = if total == 0 { 0 } else { self.find.match_ix.min(total - 1) + 1 };
        div()
            .id("find-bar")
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(IconName::Search)
            .child(div().flex_1().child(Input::new(&self.find.input).appearance(true)))
            .child(
                div()
                    .id("find-count")
                    .test_support()
                    .aria_label(format!("{current} / {total}"))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{current} / {total}")),
            )
            .child(
                Button::new("find-prev")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.find_jump(true, cx))),
            )
            .child(
                Button::new("find-next")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.find_jump(false, cx))),
            )
            .child(
                Button::new("find-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .on_click(cx.listener(|this, _, window, cx| this.open_chat_find(window, cx))),
            )
    }
}
