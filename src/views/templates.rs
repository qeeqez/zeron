//! The `/templates` picker — a dialog listing the project's saved prompt
//! templates (name + body preview). Clicking a row loads its body into the
//! composer draft — never sends; the row's hover ✕ deletes. The store lives
//! on `Workspace::templates` (see `crate::templates`).

use crate::prompts::Template;
use crate::workspace::Workspace;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Workspace {
    /// `/templates` — the template picker. The builder re-runs inside
    /// `Workspace::render`, which holds the workspace lease, so it works
    /// from a snapshot taken at open time (same rule as the file palette);
    /// a delete reopens the dialog for a fresh list (see
    /// `delete_template`).
    pub(crate) fn open_template_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let templates = self.templates.templates.clone();
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog.title("Templates").overlay_closable(true).child(
                v_flex()
                    .id("templates-list")
                    .test_support()
                    .w(px(360.))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .gap_0p5()
                    .children(template_rows(&templates, &ws, cx)),
            )
        });
    }
}

/// The composer footer's ⋯ menu — template access beside the ★ prompts
/// popover. `can_save` greys "Save as template…" on an empty draft.
pub fn templates_menu(ws: &Entity<Workspace>, can_save: bool) -> impl IntoElement {
    use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
    let ws = ws.clone();
    Button::new("composer-menu").ghost().icon(IconName::Ellipsis).dropdown_menu(move |menu, _, _| {
        let ws_open = ws.clone();
        let ws_save = ws.clone();
        menu.item(PopupMenuItem::new("Templates…").icon(IconName::LayoutTemplate).on_click(move |_, window, cx| {
            ws_open.update(cx, |this, cx| this.open_template_picker(window, cx));
        }))
        .item(
            PopupMenuItem::new("Save as template…")
                .icon(IconName::Save)
                .disabled(!can_save)
                .on_click(move |_, window, cx| {
                    ws_save.update(cx, |this, cx| this.open_save_template_dialog(window, cx));
                }),
        )
    })
}

/// The picker's rows: the muted empty state, or one row per template in
/// save order.
fn template_rows(templates: &[Template], ws: &Entity<Workspace>, cx: &App) -> Vec<AnyElement> {
    if templates.is_empty() {
        return vec![
            div()
                .id("templates-empty")
                .test_support()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No templates — save a draft as a template")
                .into_any_element(),
        ];
    }
    templates.iter().map(|t| template_row(t, ws, cx).into_any_element()).collect()
}

/// One template row: click loads the body into the composer, the hover ✕
/// deletes.
fn template_row(t: &Template, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let group = SharedString::from(format!("template-group-{}", t.name));
    let preview = t.body.lines().next().unwrap_or("").chars().take(60).collect::<String>();
    h_flex()
        .id(SharedString::from(format!("template-row-{}", t.name)))
        .test_support()
        .group(group.clone())
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .hover(|d| d.bg(cx.theme().accent))
        .child(
            // The load target is the text area only — the ✕ sits beside it
            // so its click never triggers a load.
            v_flex()
                .id(SharedString::from(format!("template-load-{}", t.name)))
                .test_support()
                .cursor_pointer()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().whitespace_nowrap().text_ellipsis().child(t.name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(preview),
                )
                .on_click({
                    let ws = ws.clone();
                    let name = t.name.clone();
                    move |_, window, cx| {
                        ws.update(cx, |this, cx| this.load_template(&name, window, cx));
                    }
                }),
        )
        .child(
            div()
                .id(SharedString::from(format!("template-delete-{}", t.name)))
                .test_support()
                .invisible()
                .group_hover(group, |style| style.visible())
                .child(
                    Button::new(SharedString::from(format!("template-delete-{}-btn", t.name)))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click({
                            let ws = ws.clone();
                            let name = t.name.clone();
                            move |_, window, cx| ws.update(cx, |this, cx| this.delete_template(&name, window, cx))
                        }),
                ),
        )
}
