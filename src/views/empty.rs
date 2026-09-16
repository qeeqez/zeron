use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The empty/new-chat state — Codex-style: a quiet brand mark, a prompt, and
/// a 2×2 grid of suggestion chips that fill the composer and send. Below the
/// chips: Open Project… plus the recent-folders list — the app's project
/// affordance when there's no conversation to look at.
///
/// First run (no usable provider, not yet skipped) swaps the whole thing for
/// the onboarding card — the condition is live, so a provider landing or a
/// Skip flips it on the next render without any extra wiring. `state` comes
/// in as a parameter: this runs inside `Workspace::render`, where the entity
/// is already mutably borrowed and `ws.read` would panic.
pub fn render_empty_state(ws: Entity<Workspace>, state: &Workspace, cx: &mut App) -> impl IntoElement {
    let show_onboarding = !state.has_configured_provider() && !state.onboarding_dismissed;
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_6()
        .child(if show_onboarding {
            onboarding_card(&ws, state.settings_panel.clone(), cx).into_any_element()
        } else {
            chat_empty(ws, state.project.root(), cx).into_any_element()
        })
}

/// The first-run setup card: app mark, one-line pitch, a primary button that
/// opens the same provider wizard Settings → Providers uses, and a ghost
/// Skip that dismisses the card permanently.
fn onboarding_card(ws: &Entity<Workspace>, panel: Entity<crate::views::settings::SettingsPanel>, cx: &App) -> impl IntoElement {
    crate::views::cards::card_frame(cx)
        .id("onboarding-card")
        .test_support()
        .items_center()
        .gap_3()
        .w(px(420.))
        .p_6()
        .child(crate::app_icon::app_icon("onboarding-icon", 40.))
        .child(div().text_lg().font_weight(FontWeight::MEDIUM).child("Welcome to Rixl Code"))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .text_center()
                .child("Connect a provider to start."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .pt_2()
                .child(Button::new("onboarding-setup").label("Set up a provider").icon(IconName::Plus).primary().on_click(
                    move |_, window, cx| {
                        panel.update(cx, |this, cx| this.open_provider_wizard(window, cx));
                    },
                ))
                .child(Button::new("onboarding-skip").label("Skip").ghost().on_click({
                    let ws = ws.clone();
                    move |_, _, cx| ws.update(cx, |this, cx| this.dismiss_onboarding(cx))
                })),
        )
}

/// The regular empty state: brand mark, prompt, suggestion chips, then the
/// project affordances (Open Project… + recent folders).
fn chat_empty(ws: Entity<Workspace>, current: &std::path::Path, cx: &mut App) -> impl IntoElement {
    let recents: Vec<std::path::PathBuf> = crate::recent_projects::list().into_iter().filter(|p| p.as_path() != current).take(5).collect();
    let suggestions = [
        "Explain this codebase",
        "Fix the failing tests",
        "Refactor the parser module",
        "Write docs for the public API",
    ];
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_6()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_10()
                .rounded_xl()
                .bg(cx.theme().secondary)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Bot).size_5()),
        )
        .child(div().text_lg().font_weight(FontWeight::MEDIUM).child("What should we work on?"))
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_center()
                .gap_2()
                .max_w(px(520.))
                .children(suggestions.iter().map(|s| {
                    let ws = ws.clone();
                    let prompt = *s;
                    div()
                        .id(SharedString::from(format!("suggestion-{prompt}")))
                        .cursor_pointer()
                        .px_3()
                        .py_1p5()
                        .rounded_full()
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .hover(|style| style.bg(cx.theme().secondary).text_color(cx.theme().foreground))
                        .child(prompt)
                        .on_click(move |_, window, cx| {
                            ws.update(cx, |this, cx| {
                                this.composer.update(cx, |s, cx| s.set_value(prompt, window, cx));
                                this.send(window, cx);
                            });
                        })
                })),
        )
        .child(
            div()
                .id("open-project")
                .test_support()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .rounded_full()
                .border_1()
                .border_color(cx.theme().border)
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().secondary).text_color(cx.theme().foreground))
                .child(Icon::new(IconName::FolderOpen).size_4())
                .child("Open Project…")
                .on_click(|_, _window, cx| crate::lifecycle::prompt_open_project(cx)),
        )
        .when(!recents.is_empty(), |d| {
            d.child(
                div()
                    .id("recent-projects")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w(px(340.))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).px_2().child("Recent projects"))
                    .children(
                        recents
                            .iter()
                            .map(|root| crate::views::project_switcher::recent_row("empty-recent", root.clone(), cx)),
                    ),
            )
        })
}
