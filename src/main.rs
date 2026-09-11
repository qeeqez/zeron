use gpui_kit::component::Root;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::*;
use gpui_kit::*;

struct Counter {
    count: i32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .justify_center()
            .items_center()
            .text_xl()
            .child(format!("Count: {}", self.count))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(Button::new("decrement").label("-").on_click(cx.listener(|this, _, _, _| this.count -= 1)))
                    .child(Button::new("increment").primary().label("+").on_click(cx.listener(|this, _, _, _| this.count += 1)))
                    .child(Button::new("reset").danger().label("Reset").on_click(cx.listener(|this, _, _, _| this.count = 0))),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| Counter { count: 0 });
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
