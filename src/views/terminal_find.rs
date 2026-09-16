//! The terminal panel's Cmd-F find bar and clickable spans. The bar is
//! per-panel — one input that always searches whichever session's tab is
//! active, so switching tabs re-runs the query for free. Matches are byte
//! ranges into `TermSession::contents()` painted as `StyledText`
//! highlights; links are `InteractiveText` click ranges over the visible
//! screen rows only — scrollback is never scanned.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chat_find::step_ix;
use crate::terminal::links::{TermLink, detect_links, find_in_lines};
use crate::workspace::Workspace;

/// Build the find input and wire its events: edits re-target the first
/// match (`term_find_query_changed`), Enter/Shift+Enter navigate
/// (`term_find_jump`) — the same contract the chat find input follows.
pub(crate) fn new_term_find_input(window: &mut Window, cx: &mut Context<Workspace>) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in terminal"));
    cx.subscribe_in(&input, window, |this, _s, event: &InputEvent, _window, cx| match event {
        InputEvent::Change => this.term_find_query_changed(cx),
        InputEvent::PressEnter { shift, .. } => this.term_find_jump(*shift, cx),
        _ => {},
    })
    .detach();
    input
}

impl Workspace {
    /// Matches for the current query against the ACTIVE session's screen —
    /// empty when the bar is closed, the query is blank, or no session
    /// exists. Recomputed per call, so a tab switch re-targets the newly
    /// active session's contents on the next render.
    pub(crate) fn term_find_matches(&self, cx: &App) -> Vec<crate::terminal::links::TermMatch> {
        if !self.terminal.find.open {
            return Vec::new();
        }
        let query = self.terminal.find.input.read(cx).value().to_string();
        self.terminal.active_session().map_or_else(Vec::new, |s| find_in_lines(&s.contents(), &query))
    }

    /// Cmd-F while the panel is focused: toggle the bar. Opening focuses
    /// the input; closing clears the query (and with it the highlights)
    /// and returns focus to the terminal's input line.
    pub(crate) fn open_terminal_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal.find.open = !self.terminal.find.open;
        self.terminal.find.match_ix = 0;
        if self.terminal.find.open {
            let input = self.terminal.find.input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        } else {
            self.terminal.find.input.update(cx, |s, cx| s.set_value("", window, cx));
            self.terminal.find.last_query.clear();
            let input = self.terminal.input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        }
        cx.notify();
    }

    /// Enter in the find bar: jump to the next match; Shift+Enter:
    /// previous. Wraps both ways like the chat find.
    pub(crate) fn term_find_jump(&mut self, back: bool, cx: &mut Context<Self>) {
        let matches = self.term_find_matches(cx);
        if matches.is_empty() {
            return;
        }
        self.terminal.find.match_ix = step_ix(self.terminal.find.match_ix, back, matches.len());
        self.scroll_to_term_match(&matches[self.terminal.find.match_ix], cx);
        cx.notify();
    }

    /// A query edit re-targets the first match. Enter also emits `Change`
    /// (the propagated keystroke inserts a `\n` that single-line mode
    /// strips), so only a real text change resets — same guard as the
    /// chat find's `last_query`.
    pub(crate) fn term_find_query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.terminal.find.input.read(cx).value().to_string();
        if query == self.terminal.find.last_query {
            return;
        }
        self.terminal.find.last_query = query;
        self.terminal.find.match_ix = 0;
        let matches = self.term_find_matches(cx);
        if let Some(first) = matches.first() {
            self.scroll_to_term_match(first, cx);
        }
        cx.notify();
    }

    /// Esc while the bar is open closes it (query cleared, highlights
    /// gone); otherwise the key falls through to the workspace's own Esc
    /// cascade.
    pub(crate) fn term_find_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.find.open {
            self.open_terminal_find(window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Bring the match's line to the top of the scroll view. Row height is
    /// approximate (the same cell metric `terminal_fit` uses) — the
    /// stronger current-match highlight marks the exact spot.
    fn scroll_to_term_match(&mut self, m: &crate::terminal::links::TermMatch, cx: &Context<Self>) {
        let line_h = f32::from(cx.theme().mono_font_size) * super::CELL_H;
        self.terminal.scroll.set_offset(point(px(0.), px(-(m.line as f32) * line_h)));
    }

    /// The find bar: query input, `n / total` readout, prev/next and
    /// close — a slimmer copy of the chat find bar's chrome.
    pub(crate) fn terminal_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.term_find_matches(cx).len();
        let current = if total == 0 { 0 } else { self.terminal.find.match_ix.min(total - 1) + 1 };
        div()
            .id("terminal-find")
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(IconName::Search)
            .child(div().flex_1().child(Input::new(&self.terminal.find.input).appearance(true)))
            .child(
                div()
                    .id("terminal-find-count")
                    .test_support()
                    .aria_label(format!("{current} / {total}"))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{current} / {total}")),
            )
            .child(
                Button::new("terminal-find-prev")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.term_find_jump(true, cx))),
            )
            .child(
                Button::new("terminal-find-next")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.term_find_jump(false, cx))),
            )
            .child(
                Button::new("terminal-find-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .on_click(cx.listener(|this, _, window, cx| this.open_terminal_find(window, cx))),
            )
    }

    /// Clickable spans in the active session's visible screen rows — the
    /// last `rows` lines of the contents, never the scrollback. `exists`
    /// stats are memoized per call so a repeated path costs one lookup per
    /// render pass.
    pub(crate) fn term_links(&self, contents: &str) -> Vec<TermLink> {
        let Some(session) = self.terminal.active_session() else {
            return Vec::new();
        };
        let rows = usize::from(session.size().0);
        let lines = contents.split('\n').count();
        let visible = lines.saturating_sub(rows)..lines;
        let root = self.project.root().to_path_buf();
        let mut cache: HashMap<std::path::PathBuf, bool> = HashMap::new();
        detect_links(contents, visible, &root, |p| *cache.entry(p.to_path_buf()).or_insert_with(|| p.exists()))
    }

    /// A Cmd-click on a link span: URLs open in the browser, file paths
    /// land in the composer as an `@path ` mention — the same as clicking
    /// the file in the explorer.
    pub(crate) fn term_link_click(&mut self, link: &TermLink, window: &mut Window, cx: &mut Context<Self>) {
        if link.is_url {
            cx.open_url(&link.target);
        } else {
            self.mention_file(&link.target, window, cx);
        }
    }
}

#[cfg(test)]
#[path = "../terminal_search_tests.rs"]
mod terminal_search_tests;
