//! The composer's saved-prompts popover: a ★ button in the footer opening a
//! list of the project's saved prompts. Clicking a row loads its text into
//! the composer; ✎ opens the rename dialog, ✕ deletes, and "Save current…"
//! names the composer's text as a new prompt. The store lives on
//! `Workspace::prompts` (see `crate::prompts`).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::*;

use crate::prompts::SavedPrompt;
use crate::workspace::Workspace;

/// Owned inputs for `saved_prompts_popover` — the composer builds this from
/// `&Workspace` so the returned element holds no borrow.
pub struct SavedPromptsSpec {
    /// Saved prompts in store order.
    pub prompts: Vec<SavedPrompt>,
    /// Whether the composer holds text worth saving — greys "Save current…".
    pub can_save: bool,
    pub ws: Entity<Workspace>,
}

/// The ★ button: opens the saved-prompts list.
pub fn saved_prompts_popover(spec: SavedPromptsSpec) -> impl IntoElement {
    Popover::new("prompts-popover")
        .anchor(Anchor::BottomRight)
        .trigger(Button::new("prompts").ghost().icon(IconName::Star))
        .content(move |_, window, cx| prompts_body(&spec, window, cx))
}

/// The popover's single column: one row per saved prompt, then the
/// save-current action.
fn prompts_body(spec: &SavedPromptsSpec, _window: &mut Window, cx: &mut Context<PopoverState>) -> AnyElement {
    let popover = cx.entity();
    let mut body = v_flex().id("prompts-list").test_support().w(px(280.)).max_h(px(320.)).overflow_y_scroll().gap_0p5();
    if spec.prompts.is_empty() {
        body = body.child(
            div()
                .id("prompts-empty")
                .test_support()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No saved prompts"),
        );
    }
    for p in &spec.prompts {
        body = body.child(prompt_row(p, &spec.ws, &popover, cx));
    }
    let ws = spec.ws.clone();
    let can_save = spec.can_save;
    body.child(
        div().id("prompt-save-current").test_support().child(
            Button::new("prompt-save-current-btn")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .label("Save current…")
                .disabled(!can_save)
                .on_click(move |_, window, cx| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    ws.update(cx, |this, cx| this.open_save_prompt_dialog(window, cx));
                }),
        ),
    )
    .into_any_element()
}

/// One saved-prompt row: click loads it into the composer, ✎ renames, ✕
/// deletes.
fn prompt_row(p: &SavedPrompt, ws: &Entity<Workspace>, popover: &Entity<PopoverState>, cx: &App) -> impl IntoElement {
    let name = p.name.clone();
    let preview = p.text.lines().next().unwrap_or("").chars().take(60).collect::<String>();
    h_flex()
        .id(SharedString::from(format!("prompt-row-{}", p.name)))
        .test_support()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .hover(|d| d.bg(cx.theme().accent))
        .child(
            // The load target is the text area only — the ✎/✕ buttons sit
            // beside it so their clicks never trigger a load.
            v_flex()
                .id(SharedString::from(format!("prompt-load-{}", p.name)))
                .test_support()
                .cursor_pointer()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().whitespace_nowrap().text_ellipsis().child(p.name.clone()))
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
                    let popover = popover.clone();
                    move |_, window, cx| {
                        ws.update(cx, |this, cx| this.load_prompt(&name, window, cx));
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                    }
                }),
        )
        .child(prompt_button(
            SharedString::from(format!("prompt-rename-{}", p.name)),
            IconName::Pencil,
            {
                let name = p.name.clone();
                move |this, window, cx| this.open_rename_prompt_dialog(&name, window, cx)
            },
            ws,
        ))
        .child(prompt_button(
            SharedString::from(format!("prompt-delete-{}", p.name)),
            IconName::X,
            {
                let name = p.name.clone();
                move |this, _window, cx| this.delete_prompt(&name, cx)
            },
            ws,
        ))
}

/// A row's icon button — `id` is the test/click target, `icon` the glyph.
/// The click stops at the button so it never triggers the row's load.
fn prompt_button(
    id: SharedString, icon: IconName, on_click: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ws: &Entity<Workspace>,
) -> impl IntoElement {
    let ws = ws.clone();
    div().id(id.clone()).test_support().child(
        Button::new(SharedString::from(format!("{id}-btn")))
            .ghost()
            .xsmall()
            .icon(icon)
            .on_click(move |_, window, cx| ws.update(cx, |this, cx| on_click(this, window, cx))),
    )
}
