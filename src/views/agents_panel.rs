use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{Agent, AgentStatus};
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_agents_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = self.agents.iter().enumerate().map(|(ix, a)| agent_card(ix, a, cx)).collect::<Vec<_>>();

        div()
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
                            .cursor_pointer()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Stop all")
                            .on_click(cx.listener(|this, _, _, cx| this.stop_all_agents(cx))),
                    )
                    .child(
                        div()
                            .id("clear-done")
                            .cursor_pointer()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_finished_agents(cx))),
                    )
                    .child(
                        div()
                            .id("close-agents")
                            .cursor_pointer()
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_agents_panel(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(cards.is_empty(), |d| d.child(div().text_sm().text_color(cx.theme().muted_foreground).child("No agents running")))
                    .children(cards),
            )
    }
}

fn agent_card(ix: usize, agent: &Agent, cx: &mut Context<Workspace>) -> AnyElement {
    let (icon, color) = match agent.status {
        AgentStatus::Running => (IconName::LoaderCircle, cx.theme().info),
        AgentStatus::Done => (IconName::CircleCheck, cx.theme().success),
        AgentStatus::Failed => (IconName::CircleX, cx.theme().danger),
        AgentStatus::Cancelled => (IconName::CircleMinus, cx.theme().muted_foreground),
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .child(div().text_color(color).child(icon))
                .child(agent.name.clone())
                .child(div().flex_1())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{}s", agent.elapsed_secs)))
                .when(agent.status == AgentStatus::Running, |d| {
                    d.child(
                        div()
                            .id(("cancel-agent", ix))
                            .cursor_pointer()
                            .text_color(cx.theme().muted_foreground)
                            .child(IconName::CircleX)
                            .on_click(cx.listener(move |this, _, _, cx| this.cancel_agent(ix, cx))),
                    )
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} · {}", agent.lane, agent.step)),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("step {}/{}", agent.steps_done, agent.steps_total)),
        )
        .into_any_element()
}
