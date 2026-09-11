mod model;
mod simulate;
mod views;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::prelude::*;
use gpui_kit::*;
use workspace::Workspace;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().size_full().child(self.render_sidebar(window, cx)).child(self.render_chat(window, cx))
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
