use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    pub fn render_composer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.chats[self.active].running;

        let send_button = if running {
            Button::new("stop").danger().icon(IconName::Pause).on_click(cx.listener(|this, _, _, cx| {
                this.chats[this.active].running = false;
                cx.notify();
            }))
        } else {
            Button::new("send")
                .primary()
                .icon(IconName::ArrowUp)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };

        div().p_3().border_t_1().border_color(cx.theme().border).child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .p_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().input)
                .child(div().flex_1().child(Textarea::new(&self.composer).appearance(false)))
                .child(send_button),
        )
    }
}
