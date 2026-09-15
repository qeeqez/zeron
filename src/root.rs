//! The `Render` impl for `Workspace` plus the window-open helper.

use crate::workspace::Workspace;
use crate::{
    Chat1, Chat2, Chat3, Chat4, Chat5, Chat6, Chat7, Chat8, Chat9, CloseWindow, CopyTranscript, DeleteChat, EmojiPalette, EnterFullscreen,
    EscapeKey, FindInChat, MinimizeWindow, NewChat, OpenPalette, OpenSettings, RecallLast, RecallNext, RecallPrev, RevealChats,
    SearchAllChats, SearchChat, ShortcutsHelp, ThemeDark, ThemeLight, ToggleAgents, ToggleChanges, ToggleDictation, ToggleExplorer,
    ToggleSidebar, ToggleSnapshots, ZoomWindow,
};
use gpui_kit::component::Root;

use gpui_kit::prelude::*;
use gpui_kit::*;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.chats[self.active].title.clone();
        window.set_window_title(&format!("{title} — Rixl Code"));
        window.set_window_edited(!self.composer.read(cx).value().is_empty());
        // Keep the sidebar's native vibrancy view in lockstep with the
        // rendered sidebar: installed only while frosted + expanded, sized to
        // sidebar_width so it tracks resize drags (no-op on headless windows).
        crate::window::sync_sidebar_vibrancy(
            window,
            self.sidebar_width,
            crate::window::sidebar_vibrancy_active(self.sidebar_frosted, self.sidebar_collapsed),
        );
        let ws_new = cx.entity();
        let ws_del = cx.entity();
        let ws_side = cx.entity();
        let ws_agents = cx.entity();
        let ws_palette = cx.entity();
        let ws_find = cx.entity();
        div()
            .key_context("workspace")
            .on_action(move |_: &NewChat, _, cx| {
                ws_new.update(cx, |this, cx| this.new_chat(cx));
            })
            .on_action(move |_: &DeleteChat, window, cx| {
                ws_del.update(cx, |this, cx| this.delete_chat(this.active, window, cx));
            })
            .on_action(chat_switch::<Chat1>(cx))
            .on_action(chat_switch::<Chat2>(cx))
            .on_action(chat_switch::<Chat3>(cx))
            .on_action(chat_switch::<Chat4>(cx))
            .on_action(chat_switch::<Chat5>(cx))
            .on_action(chat_switch::<Chat6>(cx))
            .on_action(chat_switch::<Chat7>(cx))
            .on_action(chat_switch::<Chat8>(cx))
            .on_action(chat_switch::<Chat9>(cx))
            .on_action(move |_: &ToggleSidebar, _, cx| {
                ws_side.update(cx, |this, cx| this.toggle_sidebar(cx));
            })
            .on_action(move |_: &ToggleAgents, _, cx| {
                ws_agents.update(cx, |this, cx| this.toggle_agents_panel(cx));
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ToggleChanges, _, cx| {
                    ws.update(cx, |this, cx| this.toggle_changes_panel(cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ToggleSnapshots, _, cx| {
                    ws.update(cx, |this, cx| this.toggle_snapshots_panel(cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ToggleExplorer, _, cx| {
                    ws.update(cx, |this, cx| this.toggle_explorer(cx));
                }
            })
            .on_action(move |_: &OpenPalette, window, cx| {
                ws_palette.update(cx, |this, cx| this.open_palette(window, cx));
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ThemeLight, window, cx| {
                    ws.update(cx, |this, cx| this.set_theme("light", window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ThemeDark, window, cx| {
                    ws.update(cx, |this, cx| this.set_theme("dark", window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                let handle = window.window_handle();
                move |_: &CloseWindow, window, cx| close_window(&ws, handle, window, cx)
            })
            .on_action({
                let ws = cx.entity();
                move |_: &OpenSettings, window, cx| {
                    ws.update(cx, |this, cx| this.open_settings(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &SearchChat, window, cx| {
                    ws.update(cx, |this, cx| this.open_chat_search(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &SearchAllChats, window, cx| {
                    ws.update(cx, |this, cx| this.open_global_search(window, cx));
                }
            })
            // Reached only when focus is outside the chat column — the
            // column's own FindInChat listener consumes it first.
            .on_action(move |_: &FindInChat, window, cx| ws_find.update(cx, |this, cx| this.open_chat_find(window, cx)))
            .on_action(|_: &MinimizeWindow, window, _cx| window.minimize_window())
            .on_action(|_: &ZoomWindow, window, _cx| window.zoom_window())
            .on_action(|_: &EnterFullscreen, window, _cx| window.toggle_fullscreen())
            .on_action(|_: &EmojiPalette, window, _cx| window.show_character_palette())
            .on_action(|_: &RevealChats, _window, cx| cx.reveal_path(&crate::persist::chats_dir()))
            .on_action({
                let ws = cx.entity();
                move |_: &CopyTranscript, _window, cx| {
                    ws.update(cx, |this, cx| this.copy_transcript(cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &EscapeKey, window, cx| {
                    ws.update(cx, |this, cx| this.escape(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ShortcutsHelp, window, cx| {
                    ws.update(cx, |this, cx| this.shortcuts_help(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &RecallLast, window, cx| {
                    ws.update(cx, |this, cx| this.recall_last(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &RecallPrev, window, cx| {
                    ws.update(cx, |this, cx| this.recall_prev(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &RecallNext, window, cx| {
                    ws.update(cx, |this, cx| this.recall_next(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ToggleDictation, window, cx| {
                    ws.update(cx, |this, cx| this.toggle_dictation(window, cx));
                }
            })
            .h_full()
            .relative()
            .flex()
            .flex_col()
            .on_mouse_move(cx.listener(|this, ev: &gpui_kit::MouseMoveEvent, _, cx| {
                if this.resizing_sidebar && ev.dragging() {
                    this.sidebar_width =
                        f32::from(ev.position.x).clamp(crate::window::SIDEBAR_WIDTH_MIN, crate::window::SIDEBAR_WIDTH_MAX);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(end_sidebar_drag))
            // Mouse-up outside the window still ends the drag — otherwise the
            // next hover resizes without a press.
            .on_mouse_up_out(MouseButton::Left, cx.listener(end_sidebar_drag))
            // Sidebar (full-height) + content pane. Each draws a top drag
            // strip of the same height so they read as one continuous
            // titlebar row; the traffic lights + toggle overlay the leading
            // strip's top-left.
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(!self.sidebar_collapsed, |d| d.child(self.render_sidebar(window, cx)))
                    .child(self.render_chat(window, cx))
                    .when(self.agents_panel_open, |d| d.child(self.render_agents_panel(window, cx)))
                    .when(self.changes_panel_open, |d| d.child(self.render_changes_panel(window, cx)))
                    .when(self.snapshots.open, |d| d.child(self.render_snapshots_panel(window, cx))),
            )
            // Settings is an overlay that starts at the sidebar's right edge —
            // the sidebar + toggle stay visible and functional, and the chat
            // composer keeps focus so Esc still closes settings.
            .when(self.settings_open, |d| d.child(self.settings_panel.clone()))
            // The toggle is a fixed overlay right of the traffic lights,
            // vertically centered on the top-strip line. Rendered after the
            // settings overlay so it stays visible there too — over the
            // sidebar's strip when open, the content's titlebar when
            // collapsed, and the settings top bar when settings is open.
            .child(
                div().absolute().left(px(72.)).top_0().h(px(crate::window::TOP_BAR_H)).flex().items_center().child(
                    crate::window::sidebar_toggle(self.sidebar_collapsed, cx),
                ),
            )
            // Cmd-/ cheat sheet — a centered modal over a dimmed backdrop,
            // above the sidebar toggle and settings overlay, below dialogs.
            .when(self.shortcuts_open, |d| d.child(crate::views::shortcuts::shortcuts_overlay(cx)))
            // gpui-component's Root only stores sheet/dialog/notification
            // state — the app must mount the layers itself or open_sheet /
            // open_dialog / push_notification update state nothing renders.
            .children(Root::render_notification_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
    }
}

/// Build an `on_action` handler that selects the chat at sidebar position
/// `A::IX` — matching the visible order (pinned first, then recency).
fn chat_switch<A: Action + ChatIx>(cx: &mut Context<Workspace>) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let ws = cx.entity();
    move |_: &A, window, cx| {
        ws.update(cx, |this, cx| {
            let query = this.search.read(cx).value().to_lowercase();
            if let Some(&ix) = this.sidebar_order(&query).get(A::IX) {
                this.select_chat(ix, window, cx);
            }
        });
    }
}

/// A mouse-up anywhere ends a sidebar resize drag — bound to both
/// `on_mouse_up` and `on_mouse_up_out` so a release outside the window
/// still clears `resizing_sidebar` (the next hover would resize otherwise).
fn end_sidebar_drag(this: &mut Workspace, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Workspace>) {
    if this.resizing_sidebar {
        this.resizing_sidebar = false;
        this.save_settings();
        cx.notify();
    }
}

/// Cmd+W: run the close gate (draft save + running-reply prompt), then close.
fn close_window(ws: &Entity<Workspace>, handle: AnyWindowHandle, window: &mut Window, cx: &mut App) {
    if crate::window::confirm_close(ws, handle, window, cx) {
        window.remove_window();
    }
}

trait ChatIx {
    const IX: usize;
}
macro_rules! chat_ix {
    ($($t:ident => $n:literal),*) => { $(impl ChatIx for $t { const IX: usize = $n; })* };
}
chat_ix!(Chat1 => 0, Chat2 => 1, Chat3 => 2, Chat4 => 3, Chat5 => 4, Chat6 => 5, Chat7 => 6, Chat8 => 7, Chat9 => 8);

/// Open a workspace window (used at launch, File > New Window, and dock
/// reopen). Returns the handle so callers can follow up — e.g. About opens
/// its dialog in the window it just created.
pub fn open_workspace_window(cx: &mut gpui_kit::AsyncApp) -> gpui_kit::Result<gpui_kit::WindowHandle<Root>> {
    let handle = cx.open_window(
        WindowOptions {
            window_min_size: Some(Size { width: px(800.), height: px(600.) }),
            window_bounds: crate::window::saved_window_bounds(),
            window_background: crate::appearance::window_background_appearance(crate::persist::load_settings().sidebar_frosted),
            // The app draws its own TitleBar and moves the window via
            // start_window_move, so AppKit must not treat the strip as a system
            // window-move region (which would swallow the toggle's clicks).
            app_owns_titlebar_drag: true,
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("Rixl Code".into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui_kit::point(px(9.), px(9.))),
            }),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            let ws = view.clone();
            let frosted = ws.read(cx).sidebar_frosted;
            let handle = window.window_handle();
            window.on_window_should_close(cx, move |window, cx| crate::window::confirm_close(&ws, handle, window, cx));
            cx.new(|cx| {
                let mut root = Root::new(view, window, cx);
                // Frosted sidebar needs the window's blurred background to
                // show through — Root's opaque theme fill would hide it.
                root.style().background = crate::appearance::frosted_root_background(frosted);
                root
            })
        },
    )?;
    Ok(handle)
}
