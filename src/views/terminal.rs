//! The bottom terminal panel: a real shell (see `crate::terminal`) running
//! in the project dir, its scrollback rendered monospace, plus an input
//! line that writes to the PTY. Toggled by Cmd-` / Ctrl-`, the View menu
//! and the titlebar button; open state persists in `Settings`.
//!
//! The PTY reader lives on its own thread — the panel only drains a
//! channel on a 50ms pump, so shell output never blocks a frame.

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
/// Panel chrome (header + input row + padding) subtracted before the
/// height becomes terminal rows.
const CHROME_H: f32 = 96.;

/// How a session gets its PTY — `spawn_native` in the app, a fake in
/// tests. `FnOnce` because a session is spawned at most once per panel.
pub(crate) type Spawner = Box<dyn FnOnce(&SpawnSpec) -> (Box<dyn Pty>, std::sync::mpsc::Receiver<PtyEvent>)>;

/// Terminal panel state. The session survives close/reopen (like VS Code)
/// so scrollback isn't lost; a persisted-open panel spawns its shell at
/// construction.
pub(crate) struct TerminalPanel {
    pub open: bool,
    pub session: Option<TermSession>,
    pub input: Entity<InputState>,
    pub scroll: ScrollHandle,
    /// A drain pump is in flight — `ensure_terminal` won't spawn a second.
    pump_running: bool,
    /// Test-only PTY factory — `None` spawns a real shell.
    pub(crate) spawner: Option<Spawner>,
}

impl TerminalPanel {
    pub(crate) fn new(open: bool, input: Entity<InputState>) -> Self {
        Self {
            open,
            session: None,
            input,
            scroll: ScrollHandle::new(),
            pump_running: false,
            spawner: None,
        }
    }
}

impl Workspace {
    /// Cmd-` / menu / toolbar: show or hide the panel. Opening lazily
    /// spawns the shell and focuses the input line; the state persists.
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

    /// Spawn the shell on first open and (re)start the drain pump. The
    /// pump stops when the panel closes or the session exits, so reopening
    /// must start it again even when the session already exists.
    pub(crate) fn ensure_terminal(&mut self, cx: &mut Context<Self>) {
        if self.terminal.session.is_none() {
            let (program, args) = crate::terminal::shell_spec();
            let spec = SpawnSpec {
                program,
                args,
                cwd: self.project.root().to_path_buf(),
                rows: 24,
                cols: 80,
            };
            let spawn = self.terminal.spawner.take().unwrap_or(Box::new(crate::terminal::spawn_native));
            self.terminal.session = Some(TermSession::spawn(&spec, spawn));
        }
        if !self.terminal.pump_running {
            self.terminal.pump_running = true;
            cx.spawn(async move |this, cx| terminal_pump(this, cx).await).detach();
        }
    }

    /// Pull pending PTY output into the screen. Returns `false` when the
    /// pump should stop: the panel closed or the session ended.
    fn drain_terminal(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.terminal.session.as_mut() else {
            self.terminal.pump_running = false;
            return false;
        };
        if !session.drain() {
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
    /// the PTY (what a real terminal sends) and clear the field.
    pub(crate) fn terminal_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.terminal.input.read(cx).value().to_string();
        self.terminal.input.update(cx, |s, cx| s.set_value("", window, cx));
        if let Some(session) = self.terminal.session.as_mut() {
            session.write(text.as_bytes());
            session.write(b"\r");
            self.terminal.scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    /// Fit the PTY + parser to the panel's pixel size — approximate cell
    /// metrics are fine; the shell re-wraps on the next resize.
    fn terminal_fit(&mut self, window: &Window, cx: &Context<Self>) {
        let width = f32::from(window.viewport_size().width);
        let font = f32::from(cx.theme().mono_font_size);
        let cols = (width / (font * CELL_W)) as u16;
        let rows = ((PANEL_H - CHROME_H) / (font * CELL_H)) as u16;
        if let Some(session) = self.terminal.session.as_mut() {
            session.resize(rows.clamp(4, 60), cols.clamp(20, 500));
        }
    }

    pub fn render_terminal_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.terminal_fit(window, cx);
        let contents = self.terminal.session.as_ref().map_or_else(String::new, TermSession::contents);
        let exited = self.terminal.session.as_ref().is_some_and(|s| s.exited);
        div()
            .id("terminal-panel")
            .test_support()
            .h(px(PANEL_H))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(IconName::SquareTerminal)
                    .child("Terminal")
                    .when(exited, |d| d.child("(exited)"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("close-terminal")
                            .test_support()
                            .cursor_pointer()
                            .text_color(cx.theme().muted_foreground)
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))),
                    ),
            )
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

/// Poll the session's event channel on a 50ms timer until it ends or the
/// panel closes — the same pump shape as task agents. Extracted from
/// `ensure_terminal` to keep the nesting lint happy.
async fn terminal_pump(this: WeakEntity<Workspace>, cx: &mut AsyncApp) {
    loop {
        cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
        if !this.update(cx, |this, cx| this.drain_terminal(cx)).unwrap_or(false) {
            break;
        }
    }
}
