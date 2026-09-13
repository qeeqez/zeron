//! Nav-rail rows for the settings screen: the "Back to app" closer and the
//! per-section items. Split from `settings.rs`/`settings_sections.rs` to keep
//! each file under the 250-SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::settings::{Section, SettingsPanel};

/// "Back to app" row at the top of the nav rail — closes the overlay.
pub fn back_row(ws: &Entity<crate::workspace::Workspace>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let ws = ws.clone();
    div()
        .id("settings-back")
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .mb_1()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(IconName::ArrowLeft)
        .child("Back to app")
        .on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| this.close_settings(cx));
        })
}

/// A nav-rail row for one section — icon + label, highlights when selected.
pub fn nav_item(section: Section, selected: bool, panel: &Entity<SettingsPanel>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let panel = panel.clone();
    div()
        .id(SharedString::from(format!("settings-nav-{}", section.name())))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .when(!selected, |d| d.text_color(theme.muted_foreground))
        .when(selected, |d| d.bg(theme.list_active))
        .child(section.icon())
        .child(section.label())
        .on_click(move |_, _, cx| {
            panel.update(cx, |this, cx| {
                this.section = section;
                cx.notify();
            });
        })
}
