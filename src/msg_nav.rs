//! Keyboard navigation through the transcript — Codex-style message cursor.
//! When no text input owns the keyboard, j/k (or ↑/↓) move a focus cursor
//! through the visible messages, scrolling each into view; gg/G jump to the
//! first/last message. Enter on a focused user message reopens its inline
//! editor (see `crate::chat_edit`); on any other message it copies the text.
//! Esc or a transcript click returns focus to the composer.
//!
//! The bindings live in the `workspace` key context, so they also match while
//! an input is focused — `nav_keys_allowed` gates on the *deepest* context so
//! j/k still type into the composer, find bar and dialogs (the action then
//! propagates and the keystroke falls through to text input). `gg` is a
//! double-tap on the plain `g` binding rather than a two-stroke keymap entry:
//! a pending multi-stroke binding would hold a typed `g` for the pending-input
//! timeout before inserting it.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{MessageKind, Role};
use crate::workspace::Workspace;

/// The focused transcript row. `chat_id` pins the cursor to its chat so a
/// chat switch can't highlight a same-indexed message in another thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsgNav {
    pub chat_id: u64,
    pub ix: usize,
}

/// Deepest key contexts where transcript navigation keys are allowed: the
/// workspace chrome, the window root (nothing focused), and read-only
/// `TextView`s (clicking message text focuses one). Anything else — `Input`,
/// `Command`, `List`, menus, dialogs — owns its keys, so the action
/// propagates instead of navigating.
pub(crate) fn nav_keys_allowed(window: &Window) -> bool {
    window
        .context_stack()
        .last()
        .and_then(|ctx| ctx.primary())
        .is_some_and(|entry| matches!(entry.key.as_ref(), "workspace" | "Root" | "TextView"))
}

/// Wrap a rendered message in the navigation highlight when it is the focused
/// row — a soft accent wash plus a leading accent bar, like the find hit's
/// mark but distinct from it.
pub(crate) fn wrap_nav_focus(el: AnyElement, focused: bool, cx: &App) -> AnyElement {
    if !focused {
        return el;
    }
    div()
        .id("msg-nav-focus")
        .test_support()
        .aria_label("focused message")
        .relative()
        .w_full()
        .bg(cx.theme().accent.alpha(0.08))
        .child(div().absolute().left_0().top_0().bottom_0().w(px(2.)).bg(cx.theme().accent))
        .child(el)
        .into_any_element()
}

impl Workspace {
    /// The focused message index — only while the nav focus handle is held
    /// and the cursor still points at the active chat. Focus moving to the
    /// composer, an editor or another pane hides the highlight; the stored
    /// index survives so j/k can resume from it.
    pub(crate) fn nav_target(&self, window: &Window) -> Option<usize> {
        let nav = self.nav?;
        if nav.chat_id != self.chats[self.active].id || !self.nav_focus.is_focused(window) {
            return None;
        }
        Some(nav.ix)
    }

    /// Real index of the message at visible (scroller) position `pos`, or
    /// `None` when the position is past the end. Under a chat-search filter
    /// the visible list is the matching messages; unfiltered it's identity.
    fn visible_to_real(&self, pos: usize, cx: &App) -> Option<usize> {
        let messages = &self.chats[self.active].messages;
        let query = self.chat_search.read(cx).value().to_lowercase();
        if !self.chat_search_open || query.is_empty() {
            return (pos < messages.len()).then_some(pos);
        }
        messages
            .iter()
            .enumerate()
            .filter(|(_, m)| crate::chat_search::msg_matches(m, &query))
            .nth(pos)
            .map(|(ix, _)| ix)
    }

    /// Real index of the last visible message — the entry point for j/k/Esc.
    fn last_visible(&self, cx: &App) -> Option<usize> {
        self.visible_to_real(self.filtered_count(cx).checked_sub(1)?, cx)
    }

    /// Move the focus cursor to real index `ix`, focus the transcript so the
    /// composer stops owning the keyboard, and scroll the row into view.
    fn nav_focus_ix(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.nav = Some(MsgNav { chat_id: self.chats[self.active].id, ix });
        let handle = self.nav_focus.clone();
        window.focus(&handle, cx);
        self.scroll_to_message(ix, cx);
    }

    /// j / ↓ (back=false) and k / ↑ (back=true): enter navigation at the
    /// newest message when inactive, else step through the visible rows.
    /// Without a search filter the visible list is the whole transcript, so
    /// stepping lands on adjacent messages; filtered, it lands on matches.
    pub(crate) fn nav_move(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !nav_keys_allowed(window) {
            cx.propagate();
            return;
        }
        self.pending_g = None;
        let count = self.filtered_count(cx);
        if count == 0 {
            return;
        }
        let cur = self.nav.filter(|n| n.chat_id == self.chats[self.active].id).map(|n| n.ix);
        // `filtered_pos` counts visible rows before `ix`, so it anchors even
        // when `ix` itself is filtered out; a cursor past the visible end
        // (truncated chat) restarts at the tail instead of wedging.
        let pos = cur.map(|ix| self.filtered_pos(ix, cx));
        let next = match pos {
            None => self.last_visible(cx),
            Some(p) if p >= count => self.last_visible(cx),
            Some(p) if back => p.checked_sub(1).and_then(|p| self.visible_to_real(p, cx)),
            Some(p) => (p + 1 < count).then(|| p + 1).and_then(|p| self.visible_to_real(p, cx)),
        };
        let Some(ix) = next else { return };
        self.nav_focus_ix(ix, window, cx);
        cx.notify();
    }

    /// G: focus the last visible message.
    pub(crate) fn nav_bottom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !nav_keys_allowed(window) {
            cx.propagate();
            return;
        }
        self.pending_g = None;
        let Some(ix) = self.last_visible(cx) else { return };
        self.nav_focus_ix(ix, window, cx);
        cx.notify();
    }

    /// g: a second press within the double-tap window jumps to the first
    /// visible message (vim's `gg`); a lone press only arms the prefix.
    pub(crate) fn nav_g(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !nav_keys_allowed(window) {
            cx.propagate();
            return;
        }
        const DOUBLE_TAP: std::time::Duration = std::time::Duration::from_millis(800);
        let now = std::time::Instant::now();
        if self.pending_g.is_some_and(|t| now.duration_since(t) < DOUBLE_TAP) {
            self.pending_g = None;
            if let Some(ix) = self.visible_to_real(0, cx) {
                self.nav_focus_ix(ix, window, cx);
                cx.notify();
            }
        } else {
            self.pending_g = Some(now);
        }
    }

    /// Enter on the focused message: a user text message reopens in the
    /// inline editor (edit-and-resend); anything else copies to the
    /// clipboard and keeps the cursor — like a vim yank, the position stays
    /// so j/k continue from the same row. The editor takes real focus, so
    /// the edit path clears the highlight via `nav_target`'s focus check.
    pub(crate) fn nav_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.nav_target(window) else {
            cx.propagate();
            return;
        };
        let is_user_text = self.chats[self.active]
            .messages
            .get(ix)
            .is_some_and(|m| m.role == Role::User && matches!(m.kind, MessageKind::Text(_)));
        if is_user_text {
            self.nav = None;
            self.edit_message(ix, window, cx);
        } else {
            self.copy_message(ix, cx);
        }
        cx.notify();
    }

    /// Esc in the chat column: leave navigation (focus returns to the
    /// composer); from an idle composer, enter it at the newest message —
    /// the keyboard-only way in, since transcript clicks focus the composer.
    /// Anything else (reply running, panels open, an input focused) falls
    /// through to the workspace's own Esc handling.
    pub(crate) fn nav_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_g = None;
        if self.nav_target(window).is_some() {
            self.nav = None;
            let composer = self.composer.clone();
            composer.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
            return;
        }
        let composer_focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        if self.nav_entry_idle()
            && (composer_focused || nav_keys_allowed(window))
            && let Some(ix) = self.last_visible(cx)
        {
            self.nav_focus_ix(ix, window, cx);
            cx.notify();
            return;
        }
        cx.propagate();
    }

    /// Nothing else for Esc to dismiss — mirrors `escape`'s checks so the
    /// first Esc still stops a reply or closes a panel rather than entering
    /// navigation.
    fn nav_entry_idle(&self) -> bool {
        !self.chats[self.active].running
            && !self.chat_search_open
            && !self.find.open
            && !self.settings_open
            && !self.shortcuts_open
            && !self.agents_panel_open
            && !self.changes_panel_open
            && !self.snapshots.open
            && !self.activity_open
            && self.editing.is_none()
            && self.renaming.is_none()
            && !self.chats[self.active].messages.is_empty()
    }

    /// A left press on the transcript: exit navigation and put focus back in
    /// the composer. `prevent_default` keeps the wrapper's focus-on-mousedown
    /// from grabbing the nav handle instead. When a deeper element already
    /// claimed the press (an input, a button, selectable text) it keeps
    /// focus — only the highlight clears.
    pub(crate) fn nav_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_g = None;
        if window.default_prevented() {
            if self.nav.is_some() {
                self.nav = None;
                cx.notify();
            }
            return;
        }
        self.nav = None;
        window.prevent_default();
        let composer = self.composer.clone();
        composer.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }
}
