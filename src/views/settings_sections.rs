//! Content pane bodies for each settings section — the controls that used to
//! live in the flat settings sheet, grouped by nav section.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::SelectState;
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::AccessMode;
use crate::views::settings_nav::Section;
use crate::workspace::Workspace;

/// Snapshot of workspace state + owned inputs the section bodies render from.
pub struct SettingsView {
    pub notify: bool,
    pub font_size: u8,
    pub code_font_size: u8,
    pub sidebar_frosted: bool,
    pub contrast: u16,
    pub backend: &'static str,
    pub word_wrap: bool,
    pub theme: String,
    pub ws: Entity<Workspace>,
    pub url_input: Entity<InputState>,
    pub key_input: Entity<InputState>,
    pub access: AccessMode,
    pub font_select: Entity<SelectState<Vec<String>>>,
    pub code_font_select: Entity<SelectState<Vec<String>>>,
    pub contrast_slider: Entity<SliderState>,
}

/// The content pane for the selected section.
pub fn section_body(section: Section, s: &SettingsView, cx: &App) -> impl IntoElement {
    let body = match section {
        Section::General => general_section(s, cx).into_any_element(),
        Section::Appearance => crate::views::settings_appearance::appearance_section(s, cx).into_any_element(),
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

pub(crate) fn group_label(text: &'static str, cx: &App) -> Div {
    div().pt_2().text_sm().font_semibold().text_color(cx.theme().muted_foreground).child(text)
}

fn general_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let ws = s.ws.clone();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Notifications", cx))
        .child(toggle_row(("toggle-notify", "Notify on reply complete"), s.notify, ws.clone(), |this, _w, _cx| {
            this.notify_on_done = !this.notify_on_done;
        }))
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
        .child(toggle_row(("toggle-wrap", "Word wrap"), s.word_wrap, ws.clone(), |this, _w, cx| {
            this.word_wrap = !this.word_wrap;
            this.scroller.update(cx, |s, cx| s.remeasure(cx));
        }))
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
pub(crate) fn toggle_row(
    row: (&'static str, &'static str), on: bool, ws: Entity<Workspace>, set: fn(&mut Workspace, &mut Window, &mut Context<Workspace>),
) -> impl IntoElement {
    div().flex().items_center().gap_2().text_xs().child(row.1).child(div().flex_1()).child(
        div()
            .id(row.0)
            .test_support()
            .cursor_pointer()
            .child(if on { IconName::Check } else { IconName::X })
            .on_click(move |_, window, cx| {
                ws.update(cx, |this, cx| {
                    set(this, window, cx);
                    this.save_settings();
                    cx.notify();
                });
            }),
    )
}
