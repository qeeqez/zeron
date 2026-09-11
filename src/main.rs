mod chat_ops;
mod model;
mod palette;
mod simulate;
mod views;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::component::status_bar::StatusBar;

use gpui_kit::component::theme::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use workspace::Workspace;

actions!(workspace, [NewChat, ToggleSidebar, ToggleAgents, OpenPalette, ThemeLight, ThemeDark]);

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ws_new = cx.entity();
        let ws_side = cx.entity();
        let ws_agents = cx.entity();
        let ws_palette = cx.entity();
        div()
            .key_context("workspace")
            .on_action(move |_: &NewChat, _, cx| {
                ws_new.update(cx, |this, cx| this.new_chat(cx));
            })
            .on_action(move |_: &ToggleSidebar, _, cx| {
                ws_side.update(cx, |this, cx| this.toggle_sidebar(cx));
            })
            .on_action(move |_: &ToggleAgents, _, cx| {
                ws_agents.update(cx, |this, cx| this.toggle_agents_panel(cx));
            })
            .on_action(move |_: &OpenPalette, window, cx| {
                ws_palette.update(cx, |this, cx| this.open_palette(window, cx));
            })
            .on_action(move |_: &ThemeLight, window, cx| {
                Theme::change(ThemeMode::Light, Some(window), cx);
            })
            .on_action(move |_: &ThemeDark, window, cx| {
                Theme::change(ThemeMode::Dark, Some(window), cx);
            })
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
                    .child(self.render_chat(window, cx))
                    .when(self.agents_panel_open, |d| d.child(self.render_agents_panel(window, cx))),
            )
            .child(
                StatusBar::new()
                    .left(div().text_xs().child(format!("{} · {} · rixlcode", self.model, self.mode)))
                    .right(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} chats", self.chats.len()))),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-n", NewChat, Some("workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("workspace")),
            KeyBinding::new("cmd-j", ToggleAgents, Some("workspace")),
            KeyBinding::new("cmd-k", OpenPalette, Some("workspace")),
        ]);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| Workspace::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
