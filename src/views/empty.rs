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
            onboarding_card(&ws, state, cx).into_any_element()
        } else {
            chat_empty(ws, state.project.root(), cx).into_any_element()
        })
}

/// The first-run setup card: app mark, one-line pitch, a primary button that
/// opens the same provider wizard Settings → Providers uses, and a ghost
/// Skip that dismisses the card permanently. The first render kicks the
/// provider-detection scan; once it lands the button names the first
/// detected kind ("Set up Codex CLI") instead of the generic copy.
fn onboarding_card(ws: &Entity<Workspace>, state: &Workspace, cx: &App) -> impl IntoElement {
    // First render of the card kicks the PATH/daemon scan; the deferred
    // spawn keeps the workspace borrow free until after this render.
    if state.detected_providers.is_none() && !state.detection_pending {
        let ws = ws.clone();
        cx.spawn(async move |cx| {
            ws.update(cx, |this, cx| this.detect_providers(cx));
        })
        .detach();
    }
    let detected = state.detected_providers.as_deref().unwrap_or(&[]);
    let setup_label = match detected.first() {
        Some(kind) => format!("Set up {} CLI", kind.info().label),
        None => "Set up a provider".to_string(),
    };
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
                .child(Button::new("onboarding-setup").label(setup_label).icon(IconName::Plus).primary().on_click({
                    let panel = state.settings_panel.clone();
                    move |_, window, cx| {
                        panel.update(cx, |this, cx| this.open_provider_wizard(window, cx));
                    }
                }))
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
            div().id("starter-prompts").test_support().flex().flex_col().gap_2().children(
                STARTERS
                    .chunks(2)
                    .map(|row| div().flex().gap_2().children(row.iter().map(|s| starter_chip(s, &ws, cx)))),
            ),
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

/// One starter prompt: the chip's stable test id, icon, label, and hint —
/// plus the canned text a click loads into the composer and sends. Prompts
/// stay generic about the project so they work on any open folder.
struct Starter {
    id: &'static str,
    icon: IconName,
    label: &'static str,
    hint: &'static str,
    prompt: &'static str,
}

static STARTERS: [Starter; 4] = [
    Starter {
        id: "explain",
        icon: IconName::BookOpen,
        label: "Explain this codebase",
        hint: "Tour the structure and entry points",
        prompt: "Explain the structure of this codebase and what it does.",
    },
    Starter {
        id: "fix-bug",
        icon: IconName::Bug,
        label: "Find and fix a bug",
        hint: "Hunt down a likely defect and patch it",
        prompt: "Find a likely bug in this project, explain it, and fix it.",
    },
    Starter {
        id: "add-tests",
        icon: IconName::FlaskConical,
        label: "Add tests for a file",
        hint: "Cover an untested source file",
        prompt: "Pick an important source file that lacks tests and add a test suite for it.",
    },
    Starter {
        id: "refactor",
        icon: IconName::WandSparkles,
        label: "Refactor for readability",
        hint: "Simplify a tangled spot",
        prompt: "Find a tangled part of this codebase and refactor it for readability without changing behavior.",
    },
];

/// One suggestion chip — a bordered card (the app's `card_frame` idiom)
/// with an icon, a short label, and a one-line hint. Clicking loads the
/// canned prompt into the composer and runs the normal `send` path, so
/// queueing, slash dispatch, and attachments behave exactly like typed
/// input.
fn starter_chip(s: &'static Starter, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let ws = ws.clone();
    crate::views::cards::card_frame(cx)
        .id(SharedString::from(format!("starter-{}", s.id)))
        .test_support()
        .cursor_pointer()
        .w(px(260.))
        .gap_1()
        .p_3()
        .hover(|style| style.bg(cx.theme().secondary))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::new(s.icon).size_4().text_color(cx.theme().muted_foreground))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(s.label)),
        )
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(s.hint))
        .on_click(move |_, window, cx| {
            ws.update(cx, |this, cx| {
                this.composer.update(cx, |composer, cx| composer.set_value(s.prompt, window, cx));
                this.send(window, cx);
            });
        })
}
