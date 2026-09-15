//! The Shortcuts settings section: every row of `crate::shortcuts::SHORTCUT_SPECS`
//! grouped by `ShortcutGroup`, rendered the same way the Cmd-/ cheat sheet
//! (`views::shortcuts`) draws them — description on the left, the key combo as
//! chips on the right. Both read the one table, so the section can't drift
//! from the real keymap. Split from `settings_sections.rs` to stay under the
//! 250-SLOC cap.

use gpui_kit::base::ObservedElement;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::shortcuts::{SHORTCUT_SPECS, ShortcutGroup, ShortcutSpec};
use crate::views::settings_sections::group_label;

/// The section body: one `group_label` header per `ShortcutGroup` in
/// `ShortcutGroup::ALL` order, then that group's rows.
pub(crate) fn shortcuts_section(cx: &App) -> impl IntoElement {
    let mut list = div().id("settings-shortcuts").test_support().flex().flex_col().gap_2();
    for group in ShortcutGroup::ALL {
        let specs: Vec<(usize, &ShortcutSpec)> = SHORTCUT_SPECS.iter().enumerate().filter(|(_, s)| s.group == group).collect();
        if specs.is_empty() {
            continue;
        }
        list = list.child(
            div()
                .id(SharedString::from(format!("settings-shortcuts-group-{}", group.label().to_lowercase())))
                .test_support()
                .flex()
                .flex_col()
                .gap_1()
                .child(group_label(group.label(), cx))
                .children(specs.iter().map(|(ix, spec)| shortcut_row(*ix, spec, cx))),
        );
    }
    list
}

/// One row: description on the left, the key combo as chips on the right —
/// the same layout the overlay's `shortcut_row` uses. `aria_label`s mirror
/// the visible text so headless tests can assert it.
fn shortcut_row(ix: usize, spec: &ShortcutSpec, cx: &App) -> ObservedElement<Stateful<Div>> {
    div()
        .id(("settings-shortcut-row", ix))
        .test_support()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .id(("settings-shortcut-desc", ix))
                .test_support()
                .aria_label(spec.description)
                .text_sm()
                .child(spec.description),
        )
        .child(
            div()
                .id(("settings-shortcut-keys", ix))
                .test_support()
                .aria_label(spec.keys)
                .flex()
                .items_center()
                .gap_1()
                .children(spec.keys.split('-').map(|key| chip(key, cx))),
        )
}

/// One key of a combo, rendered as a bordered kbd-style chip. Modifier names
/// become their platform glyphs (⌘⇧⌥⌃ on macOS, Ctrl/Shift/Alt/Win elsewhere).
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
