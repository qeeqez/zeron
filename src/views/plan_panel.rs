//! The Plan panel — a persistent right-side checklist of the active chat's
//! latest `PlanCard`, so the agent's plan stays glanceable after the inline
//! card scrolls away. Steps re-render live: `apply_plan` rewrites the card
//! in place and notifies, and this view reads `Workspace::active_plan`
//! fresh each frame.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{PlanStatus, PlanStep};
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_plan_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let plan = self.active_plan();
        let progress = plan.map(|p| format!("{}/{}", p.done_count(), p.steps.len()));
        let steps: Vec<AnyElement> = plan
            .map(|p| p.steps.iter().map(|s| plan_step_row(s, cx).into_any_element()).collect())
            .unwrap_or_default();

        div()
            .id("plan-panel")
            .test_support()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .font_bold()
                    .child(IconName::ListTodo)
                    .child("Plan")
                    .child(div().flex_1())
                    .when_some(progress, |d, progress| {
                        d.child(
                            div()
                                .id("plan-progress")
                                .test_support()
                                .aria_label(progress.clone())
                                .text_xs()
                                .font_normal()
                                .text_color(cx.theme().muted_foreground)
                                .child(progress),
                        )
                    })
                    .child(
                        div()
                            .id("close-plan")
                            .test_support()
                            .cursor_pointer()
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_plan_panel(cx))),
                    ),
            )
            .child(
                div()
                    .id("plan-steps")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py_1()
                    .flex()
                    .flex_col()
                    .when(steps.is_empty(), |d| {
                        d.child(
                            div()
                                .id("plan-empty")
                                .test_support()
                                .aria_label("No plan yet")
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No plan yet — the agent's checklist appears here."),
                        )
                    })
                    .children(steps),
            )
    }
}

/// One checklist row — mirrors `cards::plan_step` (same icons, colors and
/// checkbox semantics) with panel element ids so tests can address rows
/// independently of the inline card.
fn plan_step_row(step: &PlanStep, cx: &mut App) -> impl IntoElement {
    use gpui_kit::accesskit::Toggled;
    let (icon, color, toggled) = match step.status {
        PlanStatus::Done => (IconName::SquareCheck, cx.theme().success, Toggled::True),
        PlanStatus::InProgress => (IconName::LoaderCircle, cx.theme().info, Toggled::Mixed),
        PlanStatus::Pending => (IconName::Square, cx.theme().muted_foreground, Toggled::False),
    };
    div()
        .id(("plan-panel-step", step.id))
        .test_support()
        .role(gpui_kit::Role::CheckBox)
        .aria_toggled(toggled)
        .aria_label(step.label.clone())
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .child(div().text_color(color).child(icon))
        .child(
            div()
                .text_sm()
                .when(step.status == PlanStatus::Done, |d| d.line_through().text_color(cx.theme().muted_foreground))
                .when(step.status == PlanStatus::InProgress, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(step.label.clone()),
        )
}

/// The titlebar's plan toggle — icon plus the done/total count while a plan
/// exists; accent-filled while the panel is open. Extracted so
/// `chat_view.rs` stays under the SLOC cap.
pub(crate) fn plan_toggle(open: bool, progress: Option<String>, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("plan-toggle")
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_xs()
        .when(open, |d| d.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
        .when(!open, |d| d.text_color(cx.theme().muted_foreground))
        .child(IconName::ListTodo)
        .when_some(progress, |d, progress| d.child(progress))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _, _, cx| this.toggle_plan_panel(cx)))
}

/// The sidebar's Plan row — opens the panel; the suffix doubles as the
/// "plan active" indicator: the active chat's done/total count, tinted
/// while a step is still in progress. Extracted so `sidebar.rs` stays under
/// the SLOC cap.
pub(crate) fn plan_nav_row(ws: &Workspace, cx: &mut Context<Workspace>) -> super::nav_row::NavRow {
    let progress = ws
        .active_plan()
        .map(|p| (format!("{}/{}", p.done_count(), p.steps.len()), p.steps.iter().any(|s| s.status == PlanStatus::InProgress)));
    super::nav_row::NavRow::new("sidebar-plan", "Plan")
        .icon(IconName::ListTodo)
        .active(ws.plan_panel.open)
        .suffix(move |_, cx| {
            div()
                .id("sidebar-plan-progress")
                .test_support()
                .text_xs()
                .when_some(progress.clone(), |d, (text, active)| {
                    d.aria_label(text.clone())
                        .text_color(if active { cx.theme().info } else { cx.theme().muted_foreground })
                        .child(text)
                })
        })
        .on_click(cx.listener(|this, _, _, cx| this.toggle_plan_panel(cx)))
}
