//! The terminal panel's tab strip: one tab per PTY session, `+` spawns
//! another shell (capped at `MAX_TERMINAL_TABS`), `x` kills just that
//! session. Split from `terminal.rs` to keep both files under the SLOC
//! cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::terminal::{SpawnSpec, TermSession};
use crate::views::terminal::MAX_TERMINAL_TABS;
use crate::workspace::Workspace;

impl Workspace {
    /// Spawn a shell session on its own tab and make it active. No-op at
    /// the tab cap — the strip dims `+` there.
    pub(crate) fn new_terminal_tab(&mut self, cx: &mut Context<Self>) {
        if self.terminal.sessions.len() >= MAX_TERMINAL_TABS {
            return;
        }
        self.spawn_terminal_session();
        self.ensure_pump(cx);
        self.terminal.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Build a session on the user's shell in the project dir — the same
    /// spec the first tab gets — and select it.
    pub(crate) fn spawn_terminal_session(&mut self) {
        let (program, args) = crate::terminal::shell_spec();
        let spec = SpawnSpec {
            program,
            args,
            cwd: self.project.root().to_path_buf(),
            rows: 24,
            cols: 80,
        };
        let spawn = self.terminal.spawners.pop_front().unwrap_or(Box::new(crate::terminal::spawn_native));
        self.terminal.sessions.push(TermSession::spawn(&spec, spawn));
        self.terminal.active = self.terminal.sessions.len() - 1;
    }

    /// Click on a tab: bring its session to the front. An open find bar
    /// re-runs against the new session — `match_ix` resets so a stale
    /// index can't point past the new match list.
    pub(crate) fn select_terminal_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.terminal.sessions.len() || ix == self.terminal.active {
            return;
        }
        self.terminal.active = ix;
        self.terminal.find.match_ix = 0;
        self.terminal.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// `x` on a tab: drop that session (killing its shell via the PTY's
    /// `Drop`) and fix up the active index — the tab after a closed
    /// active one takes over, or the new last tab when the trail closes.
    pub(crate) fn close_terminal_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.terminal.sessions.len() {
            return;
        }
        self.terminal.sessions.remove(ix);
        if self.terminal.active > ix {
            self.terminal.active -= 1;
        } else {
            self.terminal.active = self.terminal.active.min(self.terminal.sessions.len().saturating_sub(1));
        }
        cx.notify();
    }

    /// The strip across the panel top: session tabs, `+`, then the
    /// panel's own close button pinned right.
    pub(crate) fn render_terminal_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let capped = self.terminal.sessions.len() >= MAX_TERMINAL_TABS;
        let muted_bg = cx.theme().muted;
        let muted_fg = cx.theme().muted_foreground;
        let mut tabs = Vec::with_capacity(self.terminal.sessions.len());
        for ix in 0..self.terminal.sessions.len() {
            tabs.push(terminal_tab(ix, ix == self.terminal.active, self.terminal.sessions[ix].exited, cx));
        }
        div()
            .id("terminal-tabs")
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(IconName::SquareTerminal)
            .children(tabs)
            .child(
                div()
                    .id("new-terminal-tab")
                    .test_support()
                    .cursor_pointer()
                    .rounded_md()
                    .p_1()
                    .when(capped, |d| d.opacity(0.3))
                    .when(!capped, |d| d.hover(|d| d.bg(muted_bg)))
                    .text_color(muted_fg)
                    .child(IconName::Plus)
                    .on_click(cx.listener(|this, _, _, cx| this.new_terminal_tab(cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("close-terminal")
                    .test_support()
                    .cursor_pointer()
                    .text_color(muted_fg)
                    .child(IconName::X)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))),
            )
    }
}

/// One tab: `Terminal N`, a muted `(exited)` marker once its shell dies,
/// and an `x` that closes just this session — its click must not also
/// select the tab.
fn terminal_tab(ix: usize, active: bool, exited: bool, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let label = if exited { format!("Terminal {} (exited)", ix + 1) } else { format!("Terminal {}", ix + 1) };
    let muted_bg = cx.theme().muted;
    let muted_fg = cx.theme().muted_foreground;
    div()
        .id(("terminal-tab", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .cursor_pointer()
        .when(active, |d| d.bg(muted_bg).text_color(cx.theme().foreground))
        .when(!active, |d| d.text_color(muted_fg).hover(|d| d.bg(muted_bg.opacity(0.5))))
        .child(label)
        .child(
            div()
                .id(("close-terminal-tab", ix))
                .test_support()
                .cursor_pointer()
                .text_color(muted_fg)
                .child(IconName::X)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.close_terminal_tab(ix, cx);
                })),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.select_terminal_tab(ix, cx)))
}
