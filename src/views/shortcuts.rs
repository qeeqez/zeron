//! The keyboard-shortcuts cheat sheet: a centered modal over a dimmed
//! backdrop, listing every row of `crate::shortcuts::SHORTCUT_SPECS` grouped
//! by `ShortcutGroup`. Mounted by `Workspace::render` while
//! `Workspace::shortcuts_open` is set; Esc (via `Workspace::escape`), Cmd-/,
//! the header ✕, or a backdrop click closes it.

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::shortcuts::{SHORTCUT_SPECS, ShortcutGroup, ShortcutSpec};
use crate::workspace::Workspace;

/// The overlay root: full-window backdrop + centered panel. Sibling layers —
/// the backdrop's hitbox covers the window, so a press anywhere the panel
/// doesn't occlude lands on it and closes the sheet.
pub fn shortcuts_overlay(cx: &mut Context<Workspace>) -> impl IntoElement {
    let backdrop = div()
        .id("shortcuts-backdrop")
        .test_support()
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.shortcuts_open = false;
                cx.notify();
            }),
        );
    div()
        .id("shortcuts-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .child(backdrop)
        .child(div().absolute().inset_0().flex().items_center().justify_center().child(panel(cx)))
}

/// The centered card: header, then one section per `ShortcutGroup` in
/// `ShortcutGroup::ALL` order.
fn panel(cx: &mut Context<Workspace>) -> Div {
    let theme = cx.theme();
    div()
        .occlude()
        .w(px(480.))
        .max_h(px(560.))
        .flex()
        .flex_col()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded_lg()
        .shadow_lg()
        .child(header(cx))
        .child(rows(cx))
}

fn header(cx: &mut Context<Workspace>) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(IconName::Keyboard)
                .child("Keyboard Shortcuts"),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("shortcuts-close")
                .test_support()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(IconName::X)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shortcuts_open = false;
                    cx.notify();
                })),
        )
}

/// All group sections in a scrollable column — the table outgrows small
/// windows, so the card caps its height and scrolls.
fn rows(cx: &mut Context<Workspace>) -> ObservedElement<Stateful<Div>> {
    let theme = cx.theme();
    let mut list = div().id("shortcuts-rows").test_support().flex().flex_col().gap_4().p_4().overflow_y_scroll();
    for group in ShortcutGroup::ALL {
        let specs: Vec<(usize, &ShortcutSpec)> = SHORTCUT_SPECS.iter().enumerate().filter(|(_, s)| s.group == group).collect();
        if specs.is_empty() {
            continue;
        }
        list = list.child(
            div()
                .id(SharedString::from(format!("shortcuts-group-{}", group.label().to_lowercase())))
                .test_support()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child(group.label()),
                )
                .children(specs.iter().map(|(ix, spec)| shortcut_row(*ix, spec, cx))),
        );
    }
    list
}

/// One row: description on the left, the key combo as chips on the right.
/// `aria_label`s mirror the visible text so headless tests can assert it.
fn shortcut_row(ix: usize, spec: &ShortcutSpec, cx: &App) -> ObservedElement<Stateful<Div>> {
    div()
        .id(("shortcut-row", ix))
        .test_support()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .id(("shortcut-desc", ix))
                .test_support()
                .aria_label(spec.description)
                .text_sm()
                .child(spec.description),
        )
        .child(
            div()
                .id(("shortcut-keys", ix))
                .test_support()
                .aria_label(spec.keys)
                .flex()
                .items_center()
                .gap_1()
                .children(key_segments(spec.keys).map(|key| chip(key, cx))),
        )
}

/// Split a `keys` spec into one chip per key. A trailing "-" is the key
/// itself (e.g. "cmd--" = Cmd + minus), not an empty segment.
fn key_segments(keys: &'static str) -> impl Iterator<Item = &'static str> {
    let mut parts: Vec<&str> = keys.split('-').filter(|s| !s.is_empty()).collect();
    if keys.ends_with('-') {
        parts.push("-");
    }
    parts.into_iter()
}

fn chip(key: &str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .text_xs()
        .font_family(theme.mono_font_family.clone())
        .child(display_key(key))
}

/// Display text for one `keys` segment — the keymap's lowercase names map to
/// the glyphs users see on their keyboard.
fn display_key(key: &str) -> SharedString {
    let macos = cfg!(target_os = "macos");
    match key {
        "cmd" => return if macos { "⌘".into() } else { "Ctrl".into() },
        "shift" => return if macos { "⇧".into() } else { "Shift".into() },
        "alt" => return if macos { "⌥".into() } else { "Alt".into() },
        "ctrl" => return if macos { "⌃".into() } else { "Ctrl".into() },
        "fn" => return "fn".into(),
        _ => {},
    }
    let named = match key {
        "escape" => Some(if macos { "⎋" } else { "Esc" }),
        "enter" => Some(if macos { "⏎" } else { "Enter" }),
        "backspace" => Some(if macos { "⌫" } else { "Backspace" }),
        "delete" => Some(if macos { "⌦" } else { "Delete" }),
        "up" => Some(if macos { "↑" } else { "Up" }),
        "down" => Some(if macos { "↓" } else { "Down" }),
        "left" => Some(if macos { "←" } else { "Left" }),
        "right" => Some(if macos { "→" } else { "Right" }),
        "space" => Some("Space"),
        "tab" => Some(if macos { "⇥" } else { "Tab" }),
        _ => None,
    };
    if let Some(name) = named {
        return name.into();
    }
    if key.len() == 1 {
        return key.to_uppercase().into();
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()).into(),
        None => SharedString::default(),
    }
}
