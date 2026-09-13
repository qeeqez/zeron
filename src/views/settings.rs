use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;
pub struct SettingsPanel {
    ws: Entity<Workspace>,
}

impl SettingsPanel {
    pub fn new(ws: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        cx.observe(&ws, |_, _, cx| cx.notify()).detach();
        Self { ws }
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.ws.read(cx);
        settings_body(
            SettingsView {
                notify: s.notify_on_done,
                font_size: s.font_size,
                use_codex: matches!(s.backend.name(), "codex-cli"),
                word_wrap: s.word_wrap,
                ws: self.ws.clone(),
            },
            cx,
        )
    }
}

pub struct SettingsView {
    pub notify: bool,
    pub font_size: u8,
    pub use_codex: bool,
    pub word_wrap: bool,
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
                                ws.update(cx, |this, cx| {
                                    this.notify_on_done = !this.notify_on_done;
                                    this.save_settings();
                                    cx.notify();
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
                            this.scroller.update(cx, |s, cx| s.remeasure(cx));
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
                            this.scroller.update(cx, |s, cx| s.remeasure(cx));
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
                    .on_click({
                        let ws = ws.clone();
                        move |_, _, cx| {
                            ws.update(cx, |this, cx| {
                                this.toggle_backend(cx);
                            });
                        }
                    }),
            ),
        )
        .child(div().text_sm().pt_2().child("Messages"))
        .child(
            div().flex().items_center().gap_2().text_xs().child("Word wrap").child(div().flex_1()).child(
                div()
                    .id("toggle-wrap")
                    .cursor_pointer()
                    .child(if s.word_wrap { IconName::Check } else { IconName::X })
                    .on_click({
                        let ws = ws.clone();
                        move |_, _, cx| {
                            ws.update(cx, |this, cx| {
                                this.word_wrap = !this.word_wrap;
                                this.save_settings();
                                cx.notify();
                            });
                        }
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

pub const SHORTCUTS: [(&str, &str); 14] = [
    ("Cmd+N", "New chat"),
    ("Cmd+Shift+N", "New window"),
    ("Cmd+B", "Toggle sidebar"),
    ("Cmd+J", "Toggle agents panel"),
    ("Cmd+K", "Command palette"),
    ("Cmd+F", "Search in chat"),
    ("Cmd+W", "Close window"),
    ("Cmd+,", "Settings"),
    ("Cmd+/", "Keyboard shortcuts"),
    ("Cmd+Shift+Backspace", "Delete chat"),
    ("Cmd+1..9", "Switch to chat N"),
    ("Cmd+Up", "Recall last message"),
    ("Cmd+Shift+Up/Down", "Cycle message history"),
    ("Esc", "Stop reply / close search"),
];

fn theme_button(label: &'static str, mode: ThemeMode) -> impl IntoElement {
    Button::new(SharedString::from(label)).outline().label(label).on_click(move |_, _, cx| {
        Theme::change(mode, None, cx);
    })
}
