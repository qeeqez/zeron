use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::AccessMode;
use crate::workspace::Workspace;
pub struct SettingsPanel {
    ws: Entity<Workspace>,
    url_input: Entity<InputState>,
    key_input: Entity<InputState>,
}

impl SettingsPanel {
    pub fn new(ws: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ws, |_, _, cx| cx.notify()).detach();
        let (url, key_env) = ws.read_with(cx, |s, _| (s.http_url.clone(), s.http_key_env.clone()));
        let url_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("https://…");
            s.set_value(url, window, cx);
            s
        });
        let key_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("ENV_VAR_NAME");
            s.set_value(key_env, window, cx);
            s
        });
        // Persist on every edit — the backend reads these at send time.
        for (input, field) in [(url_input.clone(), Field::Url), (key_input.clone(), Field::KeyEnv)] {
            let ctx = FieldCtx { field, ws: ws.clone() };
            cx.subscribe_in(&input, window, move |_, state, event: &InputEvent, _window, cx| {
                on_http_field(&ctx, state, event, cx);
            })
            .detach();
        }
        Self { ws, url_input, key_input }
    }
}

/// Write a changed http config field to the workspace and persist it.
fn on_http_field(ctx: &FieldCtx, state: &Entity<InputState>, event: &InputEvent, cx: &mut App) {
    if !matches!(event, InputEvent::Change) {
        return;
    }
    let value = state.read(cx).value().to_string();
    ctx.ws.update(cx, |this, _cx| {
        match ctx.field {
            Field::Url => this.http_url = value.clone(),
            Field::KeyEnv => this.http_key_env = value.clone(),
        }
        // HttpBackend clones url/key_env at construction — rebuild it so
        // the next send uses the edited values instead of the stale ones.
        if this.backend.name() == "http" {
            this.backend = std::sync::Arc::new(crate::backend::HttpBackend::new(this.http_url.clone(), this.http_key_env.clone()));
        }
        this.save_settings();
    });
}

/// Everything a field-change handler needs — bundled so the subscribe
/// closure and handler stay under the argument-count lint.
struct FieldCtx {
    field: Field,
    ws: Entity<Workspace>,
}

/// Which http config field an input writes — keeps the subscribe loop
/// under the argument-count lint.
#[derive(Clone, Copy)]
enum Field {
    Url,
    KeyEnv,
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.ws.read(cx);
        settings_body(
            SettingsView {
                notify: s.notify_on_done,
                font_size: s.font_size,
                backend: s.backend.name(),
                access: s.access(),
                word_wrap: s.word_wrap,
                ws: self.ws.clone(),
                url_input: self.url_input.clone(),
                key_input: self.key_input.clone(),
            },
            cx,
        )
    }
}

pub struct SettingsView {
    pub notify: bool,
    pub font_size: u8,
    pub backend: &'static str,
    pub word_wrap: bool,
    pub ws: Entity<Workspace>,
    pub url_input: Entity<InputState>,
    pub key_input: Entity<InputState>,
    pub access: AccessMode,
}

pub fn settings_body(s: SettingsView, _cx: &mut App) -> impl IntoElement {
    let ws = s.ws;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .child(div().text_sm().child("Appearance"))
        .child(
            div()
                .flex()
                .gap_2()
                .child(theme_button("System", "system", &ws))
                .child(theme_button("Light", "light", &ws))
                .child(theme_button("Dark", "dark", &ws)),
        )
        .child(div().text_sm().pt_2().child("Notifications"))
        .child(toggle_row(("toggle-notify", "Notify on reply complete"), s.notify, ws.clone(), |this, _cx| {
            this.notify_on_done = !this.notify_on_done;
        }))
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
                            this.font_size = this.font_size.saturating_add(1).min(24);
                            this.save_settings();
                            this.scroller.update(cx, |s, cx| s.remeasure(cx));
                            cx.notify();
                        });
                    }
                })),
        )
        .child(div().text_sm().pt_2().child("Backend"))
        .child(
            div().flex().items_center().gap_2().text_xs().child(s.backend).child(div().flex_1()).child(
                div()
                    .id("toggle-backend")
                    .cursor_pointer()
                    .text_color(hsla(0.0, 0.0, 0.55, 1.0))
                    .child("cycle")
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
        .child(div().text_sm().pt_2().child("Agent access"))
        .child(div().flex().items_center().gap_2().text_xs().children(AccessMode::ALL.into_iter().map(|mode| {
            let btn = Button::new(SharedString::from(mode.name())).label(mode.label()).on_click({
                let ws = ws.clone();
                move |_, _, cx| {
                    ws.update(cx, |this, cx| this.set_access(mode, cx));
                }
            });
            if mode == s.access { btn.primary() } else { btn.outline() }
        })))
        .child(
            div()
                .text_xs()
                .text_color(hsla(0.0, 0.0, 0.55, 1.0))
                .child("Filesystem access for Agent-mode turns — Plan and Ask always stay read-only"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .child(div().text_color(hsla(0.0, 0.0, 0.55, 1.0)).child("HTTP endpoint"))
                .child(Input::new(&s.url_input).appearance(true))
                .child(div().text_color(hsla(0.0, 0.0, 0.55, 1.0)).child("API key env var"))
                .child(Input::new(&s.key_input).appearance(true)),
        )
        .child(div().text_sm().pt_2().child("Messages"))
        .child(toggle_row(("toggle-wrap", "Word wrap"), s.word_wrap, ws.clone(), |this, cx| {
            this.word_wrap = !this.word_wrap;
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
        }))
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

/// A label + check/X row that flips a workspace flag, then persists
/// settings and re-renders — `set` does the flip plus any side effects.
/// `row` bundles the element id and label to stay under the arg-count lint.
fn toggle_row(
    row: (&'static str, &'static str), on: bool, ws: Entity<Workspace>, set: fn(&mut Workspace, &mut Context<Workspace>),
) -> impl IntoElement {
    div().flex().items_center().gap_2().text_xs().child(row.1).child(div().flex_1()).child(
        div()
            .id(row.0)
            .cursor_pointer()
            .child(if on { IconName::Check } else { IconName::X })
            .on_click(move |_, _, cx| {
                ws.update(cx, |this, cx| {
                    set(this, cx);
                    this.save_settings();
                    cx.notify();
                });
            }),
    )
}

fn theme_button(label: &'static str, mode: &'static str, ws: &Entity<Workspace>) -> impl IntoElement {
    let ws = ws.clone();
    Button::new(SharedString::from(label)).outline().label(label).on_click(move |_, window, cx| {
        ws.update(cx, |this, cx| {
            this.theme = mode.to_string();
            this.save_settings();
            this.apply_theme(window, cx);
        });
    })
}
