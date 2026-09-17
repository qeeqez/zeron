//! The Appearance settings section: theme cards, interface/code font pickers
//! and sizes, a live code preview, the contrast slider and the frosted-sidebar
//! toggle. Split from `settings_sections.rs` to stay under the 250-SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings_sections::{SettingsView, group_label, toggle_row};
use crate::workspace::Workspace;

pub(crate) fn appearance_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Theme", &s.search, cx))
        .child(
            div()
                .flex()
                .gap_3()
                .child(s.search.wrap("System", theme_card("System", "system", s, cx)))
                .child(s.search.wrap("Light", theme_card("Light", "light", s, cx)))
                .child(s.search.wrap("Dark", theme_card("Dark", "dark", s, cx))),
        )
        .child(group_label("Interface", &s.search, cx))
        .child(font_row(s, false))
        .child(group_label("Code", &s.search, cx))
        .child(font_row(s, true))
        .child(code_preview(cx))
        .child(group_label("Contrast", &s.search, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().id("contrast-slider").test_support().flex_1().child(Slider::new(&s.contrast_slider)))
                .child(div().w(px(40.)).text_xs().child(format!("{}%", s.contrast))),
        )
        .child(group_label("Sidebar", &s.search, cx))
        .child(toggle_row(
            ("toggle-sidebar-frosted", "Frosted glass sidebar"),
            s.sidebar_frosted,
            s.ws.clone(),
            |this, next, window, cx| {
                this.sidebar_frosted = next;
                this.apply_appearance(window, cx);
            },
            &s.search,
        ))
        .child(group_label("Messages", &s.search, cx))
        .child(toggle_row(
            ("toggle-compact", "Compact messages"),
            s.compact_mode,
            s.ws.clone(),
            |this, next, _w, cx| {
                this.compact_mode = next;
                // Every row's height changed — remeasure both transcript
                // scrollers (main + split pane) like the word-wrap toggle.
                this.scroller.update(cx, |s, cx| s.remeasure(cx));
                this.secondary_scroller.update(cx, |s, cx| s.remeasure(cx));
            },
            &s.search,
        ))
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
            ws.update(cx, |this, cx| this.set_theme(mode, window, cx));
        })
}

/// Font family picker + size stepper for one text role. `code` selects the
/// code (mono) fields; `false` the interface fields.
fn font_row(s: &SettingsView, code: bool) -> AnyElement {
    let (id, select, size, placeholder) = if code {
        ("code-font-select", &s.code_font_select, s.code_font_size, "Default mono font")
    } else {
        ("font-select", &s.font_select, s.font_size, "System font")
    };
    s.search.wrap(
        placeholder,
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Select::new(select).id(id).small().appearance(true).cleanable(true).placeholder(placeholder)),
            )
            .child(size_stepper(id, size, s.ws.clone(), code)),
    )
}

/// −/+ stepper around the current size; writes the role's font size, persists
/// and re-applies the theme so the change shows immediately. The size label
/// carries an id + aria-label so headless tests see zoom-shortcut changes.
/// Interface text steps in half-points; code in whole px.
fn size_stepper(id: &'static str, size: f32, ws: Entity<Workspace>, code: bool) -> impl IntoElement {
    use crate::appearance::FONT_SIZE_STEP;
    let step = move |sign: f32, window: &mut Window, cx: &mut App| {
        ws.update(cx, |this, cx| {
            let (cur, inc) = if code { (this.code_font_size, 1.) } else { (this.font_size, FONT_SIZE_STEP) };
            this.set_font_size(cur + sign * inc, code, window, cx);
        });
    };
    let dec = step.clone();
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(
            div()
                .id(SharedString::from(format!("{id}-dec")))
                .test_support()
                .cursor_pointer()
                .child(IconName::Minus)
                .on_click(move |_, window, cx| dec(-1., window, cx)),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-size")))
                .test_support()
                .aria_label(format!("{size}px"))
                .child(format!("{size}px")),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-inc")))
                .test_support()
                .cursor_pointer()
                .child(IconName::Plus)
                .on_click(move |_, window, cx| step(1., window, cx)),
        )
}

/// A rendered code sample in the selected mono family + size — the same
/// `theme.mono_font_*` fields the markdown code blocks read.
fn code_preview(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (kw, func, str_, ty, fg, comment) = (theme.magenta, theme.blue, theme.green, theme.cyan, theme.foreground, theme.muted_foreground);
    div()
        .id("code-preview")
        .test_support()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .p_3()
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .flex()
        .flex_col()
        .child(code_line(&[("// how your code will look", comment)]))
        .child(code_line(&[("fn ", kw), ("main", func), ("() {", fg)]))
        .child(code_line(&[("    let ", kw), ("msg", fg), (" = ", fg), ("\"Hello, world\"", str_), (";", fg)]))
        .child(code_line(&[("    println!", fg), ("(\"{msg}\")", str_), (";", fg)]))
        .child(code_line(&[("}", fg)]))
        .child(code_line(&[("struct ", kw), ("Size", ty), (" { ", fg), ("w", fg), (": ", fg), ("u8", ty), (" }", fg)]))
}

/// One preview line: colored spans laid out inline.
fn code_line(parts: &[(&'static str, Hsla)]) -> Div {
    div().flex().children(parts.iter().map(|(text, color)| div().text_color(*color).child(*text)))
}
