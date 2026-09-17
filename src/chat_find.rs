//! In-chat find (Cmd-F): a Codex-style find bar over the transcript. Unlike
//! `chat_search` — which filters the message list down to matches — find
//! keeps every message mounted, highlights the ones containing the query and
//! scrolls to the current match on Enter / Shift+Enter.
//!
//! A "match" is a message whose text contains the query under the bar's
//! `FindOpts` (case-insensitive substring by default; the Match Case and
//! Whole Word chips narrow it). Navigation and the `n / total` readout
//! count matching messages, which is also the granularity of the highlight.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chat_search::find_opts::FindOpts;
use crate::chat_search::role_filter::RoleFilter;
use crate::model::ChatMessage;
use crate::workspace::Workspace;

/// Indices of messages matching `query`, in transcript order. An empty query
/// matches nothing — the bar shows `0 / 0` rather than highlighting all.
/// `role` narrows the hits to one side of the conversation (`All` keeps
/// both); `opts` applies the Match Case / Whole Word toggles.
pub(crate) fn matching_messages(messages: &[ChatMessage], query: &str, role: RoleFilter, opts: FindOpts) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    (0..messages.len())
        .filter(|&ix| role.matches(messages[ix].role) && opts.msg_matches(&messages[ix], query))
        .collect()
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
/// `find_query_changed` compares before resetting `match_ix`. `role` is the
/// All / You / Assistant toggle; it resets to `All` whenever the bar closes.
/// `opts` holds the Match Case / Whole Word chips — unlike `role` they stay
/// set across open/close, the way editor find bars keep them sticky for the
/// session (they're never persisted to disk).
pub(crate) struct FindBar {
    pub input: Entity<InputState>,
    pub open: bool,
    /// Index of the current match within `find_matches` — drives the
    /// `n / total` readout and the stronger highlight on the current hit.
    pub match_ix: usize,
    pub last_query: String,
    pub role: RoleFilter,
    pub opts: FindOpts,
}

impl FindBar {
    pub(crate) fn new(input: Entity<InputState>) -> Self {
        Self {
            input,
            open: false,
            match_ix: 0,
            last_query: String::new(),
            role: RoleFilter::All,
            opts: FindOpts::default(),
        }
    }
}

impl Workspace {
    /// Matching message indices for the current find query and role filter —
    /// empty when the bar is closed or the query is blank.
    pub(crate) fn find_matches(&self, cx: &App) -> Vec<usize> {
        if !self.find.open {
            return Vec::new();
        }
        let query = self.find.input.read(cx).value().to_string();
        matching_messages(&self.chats[self.active].messages, &query, self.find.role, self.find.opts)
    }

    /// Cmd-F / FindInChat: toggle the find bar. Opening focuses the input;
    /// closing clears the query and returns focus to the composer. The role
    /// filter resets to `All` on either transition, so a reopened bar always
    /// starts unfiltered — the Match Case / Whole Word chips instead stay
    /// sticky for the session, like an editor's find bar.
    pub fn open_chat_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = !self.find.open;
        self.find.match_ix = 0;
        self.find.role = RoleFilter::All;
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

    /// Re-target the first surviving match after a narrowing change (role
    /// cycle, Match Case / Whole Word flip) — the same reset a query edit
    /// performs, minus the `last_query` bookkeeping that guards it.
    fn find_retarget(&mut self, cx: &mut Context<Self>) {
        self.find.match_ix = 0;
        let matches = self.find_matches(cx);
        if let Some(&first) = matches.first() {
            self.scroll_to_message(first, cx);
        }
        cx.notify();
    }

    /// The role toggle's click: cycle All → You → Assistant and re-target the
    /// first surviving match, like a query edit.
    pub(crate) fn find_role_cycle(&mut self, cx: &mut Context<Self>) {
        self.find.role = self.find.role.cycle();
        self.find_retarget(cx);
    }

    /// The Match Case chip: flip exact-case matching and re-run the current
    /// needle immediately, re-targeting the first surviving hit.
    pub(crate) fn find_match_case_toggle(&mut self, cx: &mut Context<Self>) {
        self.find.opts.case_sensitive = !self.find.opts.case_sensitive;
        self.find_retarget(cx);
    }

    /// The Whole Word chip: flip word-boundary matching and re-run the
    /// current needle immediately, re-targeting the first surviving hit.
    pub(crate) fn find_whole_word_toggle(&mut self, cx: &mut Context<Self>) {
        self.find.opts.whole_word = !self.find.opts.whole_word;
        self.find_retarget(cx);
    }

    /// Open the find bar on `query` and land on message `msg_ix` — the
    /// global-search jump target. `last_query` is pre-seeded so the
    /// programmatic `set_value`'s Change event doesn't reset `match_ix`
    /// back to the first match (see `find_query_changed`). A role filter
    /// or find toggles that would hide the target widen back to their
    /// unconstrained defaults — the jump must land on the confirmed hit.
    pub(crate) fn jump_to_message(&mut self, query: &str, msg_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.open {
            self.open_chat_find(window, cx);
        }
        let target = self.chats[self.active].messages.get(msg_ix);
        if target.is_some_and(|m| !self.find.role.matches(m.role)) {
            self.find.role = RoleFilter::All;
        }
        // Global search always matches case-insensitively — a Match Case /
        // Whole Word pair strict enough to hide the confirmed hit resets.
        if target.is_some_and(|m| !self.find.opts.msg_matches(m, query)) {
            self.find.opts = FindOpts::default();
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
}

// The find bar's element lives in `views::chat_find_bar` — this file is at
// the SLOC cap.

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "chat_find_role_tests.rs"]
mod chat_find_role_tests;

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "chat_find_toggle_tests.rs"]
mod chat_find_toggle_tests;
