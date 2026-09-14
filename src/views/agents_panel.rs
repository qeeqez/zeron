use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::AgentStatus;
use crate::workspace::Workspace;

use super::agents_card::agent_card;

impl Workspace {
    pub fn render_agents_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running: Vec<AnyElement> = self
            .agents
            .iter()
            .filter(|a| a.status == AgentStatus::Running)
            .map(|a| agent_card(a, crate::agents::agent_tools(self, a), cx))
            .collect();
        let finished: Vec<AnyElement> = self
            .agents
            .iter()
            .filter(|a| a.status != AgentStatus::Running)
            .map(|a| agent_card(a, crate::agents::agent_tools(self, a), cx))
            .collect();

        div()
            .id("agents-panel")
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
                    .child(IconName::Bot)
                    .child("Agents")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("stop-all")
                            .test_support()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Stop all")
                            .on_click(cx.listener(|this, _, _, cx| this.stop_all_agents(cx))),
                    )
                    .child(
                        div()
                            .id("clear-done")
                            .test_support()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_finished_agents(cx))),
                    )
                    .child(
                        div()
                            .id("close-agents")
                            .test_support()
                            .cursor_pointer()
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_agents_panel(cx))),
                    ),
            )
            .child(
                div()
                    .id("agents-list")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(running.is_empty() && finished.is_empty(), |d| {
                        d.child(div().text_sm().text_color(cx.theme().muted_foreground).child("No agents running"))
                    })
                    .children(running)
                    .when(!finished.is_empty(), |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .pt_2()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .child("Finished"),
                        )
                        .children(finished)
                    }),
            )
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(gpui_kit::component::input::Input::new(&self.task_input).appearance(true)),
            )
    }
}
