mod model;
mod simulate;
mod views;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;
use workspace::Workspace;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
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
                    .left(div().text_xs().child("gpt-5-codex · rixlcode"))
                    .right(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} chats", self.chats.len()))),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
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
