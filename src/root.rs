//! The `Render` impl for `Workspace` — window-opening lives in `lifecycle`,
//! the `on_action` chain in `root_actions` (split for the SLOC cap).

use crate::workspace::Workspace;
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
        crate::root_actions::workspace_actions(div().key_context("workspace"), window, cx)
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
                    .when(self.plan_panel.open, |d| d.child(self.render_plan_panel(window, cx)))
                    .when(self.snapshots.open, |d| d.child(self.render_snapshots_panel(window, cx))),
            )
            // Bottom terminal panel — full width below the sidebar + chat
            // row, like Codex/VS Code.
            .when(self.terminal.open, |d| d.child(self.render_terminal_panel(window, cx)))
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
            // Activity center: bell + dropdown as one overlay layer so the
            // panel floats above the sidebar and chat pane.
            .child(crate::views::activity::activity_overlay(self, cx))
            // Cmd-/ cheat sheet — a centered modal over a dimmed backdrop,
            // above the sidebar toggle and settings overlay, below dialogs.
            .when(self.shortcuts_open, |d| d.child(crate::views::shortcuts::shortcuts_overlay(cx)))
            // View Logs — same centered-modal shape as the cheat sheet,
            // layered above it so Esc dismisses logs first.
            .when(self.logs_open, |d| d.child(crate::views::logs::logs_overlay(self, cx)))
            // Usage dashboard — same centered-modal shape, layered above
            // logs so Esc dismisses it first.
            .when(self.usage_dashboard_open, |d| d.child(crate::views::usage_dashboard::usage_dashboard_overlay(self, cx)))
            // gpui-component's Root only stores sheet/dialog/notification
            // state — the app must mount the layers itself or open_sheet /
            // open_dialog / push_notification update state nothing renders.
            .children(Root::render_notification_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            // Image lightbox — topmost layer: a click on an image thumbnail
            // shows it full-size over everything, dialogs included.
            .when_some(self.image_view.clone(), |d, path| {
                d.child(crate::image_view::image_view_overlay(&path, window, cx))
            })
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
