//! The bottom terminal panel: real shells (see `crate::terminal`) running
//! in the project dir, one tab per session (see `terminal_tabs`), the
//! active tab's scrollback rendered monospace, plus an input line that
//! writes to its PTY. Toggled by Cmd-` / Ctrl-`, the View menu and the
//! titlebar button; open state persists in `Settings`.
//!
//! Each PTY reader lives on its own thread — the panel only drains the
//! sessions' channels on a 50ms pump, so shell output never blocks a
//! frame. Background tabs keep draining while another is active, so no
//! output is lost on a switch.

use std::collections::VecDeque;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::terminal::{Pty, PtyEvent, SpawnSpec, TermSession};
use crate::workspace::Workspace;

/// Fixed panel height — the chat column keeps the rest.
const PANEL_H: f32 = 240.;
/// Approximate monospace cell metrics — the PTY size only needs to be
/// close; the shell re-wraps on the next resize.
const CELL_W: f32 = 0.6;
const CELL_H: f32 = 1.35;
/// Panel chrome (tab strip + input row + padding) subtracted before the
/// height becomes terminal rows.
const CHROME_H: f32 = 96.;
/// Tab ceiling — the strip stays a single row and shells aren't free.
pub(crate) const MAX_TERMINAL_TABS: usize = 8;

/// How a session gets its PTY — `spawn_native` in the app, a fake in
/// tests. `FnOnce` because each queued spawner builds exactly one session.
pub(crate) type Spawner = Box<dyn FnOnce(&SpawnSpec) -> (Box<dyn Pty>, std::sync::mpsc::Receiver<PtyEvent>)>;
/// Terminal panel state. Sessions survive close/reopen (like VS Code) so
/// scrollback isn't lost; a persisted-open panel spawns its first shell at
/// construction.
pub(crate) struct TerminalPanel {
    pub open: bool,
    /// Live sessions, one per tab. `active` indexes the one on screen.
    pub sessions: Vec<TermSession>,
    pub active: usize,
    pub input: Entity<InputState>,
    pub scroll: ScrollHandle,
    /// A drain pump is in flight — `ensure_pump` won't spawn a second.
    pump_running: bool,
    /// Test-only PTY factories, one per session to spawn — an empty queue
    /// spawns a real shell.
    pub(crate) spawners: VecDeque<Spawner>,
}

impl TerminalPanel {
    pub(crate) fn new(open: bool, input: Entity<InputState>) -> Self {
        Self {
            open,
            sessions: Vec::new(),
            active: 0,
            input,
            scroll: ScrollHandle::new(),
            pump_running: false,
            spawners: VecDeque::new(),
        }
    }

    /// The session on screen — what the input line writes to and the
    /// scroll view renders.
    pub(crate) fn active_session(&self) -> Option<&TermSession> {
        self.sessions.get(self.active)
    }

    fn active_session_mut(&mut self) -> Option<&mut TermSession> {
        self.sessions.get_mut(self.active)
    }
}

impl Workspace {
    /// Cmd-` / menu / toolbar: show or hide the panel. Opening lazily
    /// spawns the first shell and focuses the input line; the state
    /// persists.
    pub fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal.open = !self.terminal.open;
        if self.terminal.open {
            self.ensure_terminal(cx);
            let input = self.terminal.input.clone();
            input.update(cx, |s, cx| s.focus(window, cx));
            self.terminal.scroll.scroll_to_bottom();
        }
        self.save_settings();
        cx.notify();
    }

    /// Spawn the first shell when none exist and (re)start the drain
    /// pump. The pump stops when the panel closes or every session has
    /// exited, so reopening must start it again even when sessions
    /// already exist.
    pub(crate) fn ensure_terminal(&mut self, cx: &mut Context<Self>) {
        if self.terminal.sessions.is_empty() {
            self.spawn_terminal_session();
        }
        self.ensure_pump(cx);
    }

    /// Start the 50ms drain pump unless one is already in flight.
    pub(crate) fn ensure_pump(&mut self, cx: &mut Context<Self>) {
        if !self.terminal.pump_running {
            self.terminal.pump_running = true;
            cx.spawn(async move |this, cx| terminal_pump(this, cx).await).detach();
        }
    }

    /// Pull pending PTY output into every session — background tabs keep
    /// collecting while another renders. Returns `false` when the pump
    /// should stop: the panel closed or no session is still live.
    fn drain_terminal(&mut self, cx: &mut Context<Self>) -> bool {
        let mut any_live = false;
        for session in &mut self.terminal.sessions {
            any_live |= session.drain();
        }
        if self.terminal.sessions.is_empty() || !any_live {
            self.terminal.pump_running = false;
            cx.notify();
            return false;
        }
        // Follow the tail only while the user is already at the bottom —
        // output shouldn't yank the view away from scrollback they're
        // reading.
        let at_bottom = self.terminal.scroll.offset().y <= -self.terminal.scroll.max_offset().y;
        if at_bottom {
            self.terminal.scroll.scroll_to_bottom();
        }
        cx.notify();
        self.terminal.open
    }

    /// Enter in the input line: write the text plus a carriage return to
    /// the active PTY (what a real terminal sends) and clear the field.
    pub(crate) fn terminal_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.terminal.input.read(cx).value().to_string();
        self.terminal.input.update(cx, |s, cx| s.set_value("", window, cx));
        if let Some(session) = self.terminal.active_session_mut() {
            session.write(text.as_bytes());
            session.write(b"\r");
            self.terminal.scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    /// Fit every PTY + parser to the panel's pixel size — approximate
    /// cell metrics are fine; the shells re-wrap on the next resize.
    fn terminal_fit(&mut self, window: &Window, cx: &Context<Self>) {
        let width = f32::from(window.viewport_size().width);
        let font = f32::from(cx.theme().mono_font_size);
        let cols = (width / (font * CELL_W)) as u16;
        let rows = ((PANEL_H - CHROME_H) / (font * CELL_H)) as u16;
        for session in &mut self.terminal.sessions {
            session.resize(rows.clamp(4, 60), cols.clamp(20, 500));
        }
    }

    pub fn render_terminal_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.terminal_fit(window, cx);
        let contents = self.terminal.active_session().map_or_else(String::new, TermSession::contents);
        div()
            .id("terminal-panel")
            .test_support()
            .h(px(PANEL_H))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(self.render_terminal_tabs(cx))
            .child(
                div()
                    .id("terminal-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .py_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.terminal.scroll)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .whitespace_nowrap()
                    .child(contents),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(IconName::ChevronRight)
                    .child(div().flex_1().child(Input::new(&self.terminal.input).appearance(true))),
            )
    }
}

/// Poll the sessions' event channels on a 50ms timer until they all end
/// or the panel closes — the same pump shape as task agents.
async fn terminal_pump(this: WeakEntity<Workspace>, cx: &mut AsyncApp) {
    loop {
        cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
        if !this.update(cx, |this, cx| this.drain_terminal(cx)).unwrap_or(false) {
            break;
        }
    }
}

#[cfg(test)]
#[path = "../terminal_tabs_tests.rs"]
mod terminal_tabs_tests;
