use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;
pub struct SettingsView {
    pub notify: bool,
    pub font_size: u8,
    pub use_codex: bool,
    pub ws: Entity<Workspace>,
}

pub fn settings_body(s: SettingsView, _cx: &mut App) -> impl IntoElement {
    let ws = s.ws;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .child(div().text_sm().child("Theme"))
        .child(
            div()
                .flex()
                .gap_2()
                .child(theme_button("Light", ThemeMode::Light))
                .child(theme_button("Dark", ThemeMode::Dark)),
        )
        .child(div().text_sm().pt_2().child("Notifications"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child("Notify on reply complete")
                .child(div().flex_1())
                .child(
                    div()
                        .id("toggle-notify")
                        .cursor_pointer()
                        .child(if s.notify { IconName::Check } else { IconName::X })
                        .on_click({
                            let ws = ws.clone();
                            move |_, _, cx| {
                                ws.update(cx, |this, _cx| {
                                    this.notify_on_done = !this.notify_on_done;
                                    this.save_settings();
                                });
                            }
                        }),
                ),
        )
        .child(div().text_sm().pt_2().child("Font size"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(div().id("font-dec").cursor_pointer().child(IconName::Minus).on_click({
                    let ws = ws.clone();
                    move |_, _, cx| {
                        ws.update(cx, |this, cx| {
                            this.font_size = this.font_size.saturating_sub(1).max(10);
                            this.save_settings();
                            cx.notify();
                        });
                    }
                }))
                .child(format!("{}px", s.font_size))
                .child(div().id("font-inc").cursor_pointer().child(IconName::Plus).on_click({
                    let ws = ws.clone();
                    move |_, _, cx| {
                        ws.update(cx, |this, cx| {
                            this.font_size = (this.font_size + 1).min(24);
                            this.save_settings();
                            cx.notify();
                        });
                    }
                })),
        )
        .child(div().text_sm().pt_2().child("Backend"))
        .child(
            div().flex().items_center().gap_2().text_xs().child("Use codex CLI").child(div().flex_1()).child(
                div()
                    .id("toggle-backend")
                    .cursor_pointer()
                    .child(if s.use_codex { IconName::Check } else { IconName::X })
                    .on_click(move |_, _, cx| {
                        ws.update(cx, |this, cx| {
                            this.toggle_backend(cx);
                        });
                    }),
            ),
        )
        .child(div().text_sm().pt_2().child("Shortcuts"))
        .child(div().flex().flex_col().gap_1().text_xs().children(SHORTCUTS.iter().map(|(key, desc)| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(140.)).child(*key))
                .child(div().text_color(hsla(0.0, 0.0, 0.55, 1.0)).child(*desc))
        })))
}

const SHORTCUTS: [(&str, &str); 8] = [
    ("Cmd+N", "New chat"),
    ("Cmd+B", "Toggle sidebar"),
    ("Cmd+J", "Toggle agents panel"),
    ("Cmd+K", "Command palette"),
    ("Cmd+W", "Close window"),
    ("Cmd+,", "Settings"),
    ("Cmd+Shift+Backspace", "Delete chat"),
    ("Cmd+1..9", "Switch to chat N"),
];

fn theme_button(label: &'static str, mode: ThemeMode) -> impl IntoElement {
    Button::new(SharedString::from(label)).outline().label(label).on_click(move |_, _, cx| {
        Theme::change(mode, None, cx);
    })
}
