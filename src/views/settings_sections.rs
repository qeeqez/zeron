//! Content pane bodies for each settings section — the controls that used to
//! live in the flat settings sheet, grouped by nav section.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::AccessMode;
use crate::views::settings::Section;
use crate::workspace::Workspace;

/// Snapshot of workspace state + owned inputs the section bodies render from.
pub struct SettingsView {
    pub notify: bool,
    pub font_size: u8,
    pub backend: &'static str,
    pub word_wrap: bool,
    pub theme: String,
    pub ws: Entity<Workspace>,
    pub url_input: Entity<InputState>,
    pub key_input: Entity<InputState>,
    pub access: AccessMode,
}

/// The content pane for the selected section.
pub fn section_body(section: Section, s: &SettingsView, cx: &App) -> impl IntoElement {
    let body = match section {
        Section::General => general_section(s, cx).into_any_element(),
        Section::Appearance => appearance_section(s, cx).into_any_element(),
        Section::Shortcuts => shortcuts_section(cx).into_any_element(),
        Section::Voice => placeholder_section("Voice input and dictation are not configured yet.", cx),
        Section::Profile => placeholder_section("Signed in as a local account — no profile to manage.", cx),
        Section::McpServers => placeholder_section("No MCP servers configured.", cx),
    };
    div()
        .id(SharedString::from(format!("settings-section-{}", section.name())))
        .test_support()
        .flex()
        .flex_col()
        .gap_4()
        .child(body)
}

fn placeholder_section(text: &'static str, cx: &App) -> AnyElement {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}

fn group_label(text: &'static str, cx: &App) -> Div {
    div().pt_2().text_sm().font_semibold().text_color(cx.theme().muted_foreground).child(text)
}

fn general_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Notifications", cx))
        .child(toggle_row(("toggle-notify", "Notify on reply complete"), s.notify, ws.clone(), |this, _cx| {
            this.notify_on_done = !this.notify_on_done;
        }))
        .child(group_label("Font size", cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(div().id("font-dec").test_support().cursor_pointer().child(IconName::Minus).on_click({
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
                .child(div().id("font-inc").test_support().cursor_pointer().child(IconName::Plus).on_click({
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
        .child(group_label("Backend", cx))
        .child(
            div().flex().items_center().gap_2().text_xs().child(s.backend).child(div().flex_1()).child(
                div()
                    .id("toggle-backend")
                    .test_support()
                    .cursor_pointer()
                    .text_color(cx.theme().muted_foreground)
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
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .child(div().text_color(cx.theme().muted_foreground).child("HTTP endpoint"))
                .child(Input::new(&s.url_input).appearance(true))
                .child(div().text_color(cx.theme().muted_foreground).child("API key env var"))
                .child(Input::new(&s.key_input).appearance(true)),
        )
        .child(group_label("Agent access", cx))
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
                .text_color(cx.theme().muted_foreground)
                .child("Filesystem access for Agent-mode turns — Plan and Ask always stay read-only"),
        )
        .child(group_label("Messages", cx))
        .child(toggle_row(("toggle-wrap", "Word wrap"), s.word_wrap, ws.clone(), |this, cx| {
            this.word_wrap = !this.word_wrap;
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
        }))
}

fn appearance_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    div().flex().flex_col().gap_3().child(group_label("Theme", cx)).child(
        div()
            .flex()
            .gap_3()
            .child(theme_card("System", "system", s, cx))
            .child(theme_card("Light", "light", s, cx))
            .child(theme_card("Dark", "dark", s, cx)),
    )
}

/// A Codex-style theme card: mini preview swatch over a label, accent border
/// when active.
fn theme_card(label: &'static str, mode: &'static str, s: &SettingsView, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let selected = s.theme == mode;
    let (preview_bg, preview_fg) = match mode {
        "light" => (hsla(0.0, 0.0, 0.98, 1.0), hsla(0.0, 0.0, 0.2, 1.0)),
        "dark" => (hsla(0.0, 0.0, 0.12, 1.0), hsla(0.0, 0.0, 0.85, 1.0)),
        _ => (theme.background, theme.foreground),
    };
    let ws = s.ws.clone();
    div()
        .id(SharedString::from(format!("theme-{mode}")))
        .test_support()
        .flex()
        .flex_col()
        .gap_2()
        .w(px(140.))
        .cursor_pointer()
        .child(
            div()
                .h(px(72.))
                .w_full()
                .rounded_md()
                .border_1()
                .border_color(if selected { theme.list_active_border } else { theme.border })
                .bg(preview_bg)
                .p_2()
                .child(div().w(px(48.)).h(px(6.)).rounded_sm().bg(preview_fg)),
        )
        .child(div().text_xs().child(label))
        .on_click(move |_, window, cx| {
            ws.update(cx, |this, cx| {
                this.theme = mode.to_string();
                this.save_settings();
                this.apply_theme(window, cx);
            });
        })
}

fn shortcuts_section(cx: &App) -> impl IntoElement {
    div().flex().flex_col().gap_1().text_xs().children(SHORTCUTS.iter().map(|(key, desc)| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(px(160.)).font_weight(FontWeight::SEMIBOLD).child(*key))
            .child(div().text_color(cx.theme().muted_foreground).child(*desc))
    }))
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
            .test_support()
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
