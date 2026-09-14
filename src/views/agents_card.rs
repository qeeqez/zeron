//! One agent card in the agents panel: status header, progress line,
//! per-tool rows with expandable output, and the event log.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{Agent, AgentStatus, ToolCall, ToolStatus};
use crate::workspace::Workspace;

pub(super) fn agent_card(agent: &Agent, tools: Vec<&ToolCall>, cx: &mut Context<Workspace>) -> AnyElement {
    let (icon, color) = match agent.status {
        AgentStatus::Running => (IconName::LoaderCircle, cx.theme().info),
        AgentStatus::Done => (IconName::CircleCheck, cx.theme().success),
        AgentStatus::Failed => (IconName::CircleX, cx.theme().danger),
        AgentStatus::Cancelled => (IconName::CircleMinus, cx.theme().muted_foreground),
    };
    let id = agent.id;
    // Running rows show just the clock — the spinner already says running.
    // Finished rows lead with the outcome so done/failed/cancelled read at
    // a glance, like Codex's "Worked for Ns" summary.
    let timing = if agent.status == AgentStatus::Running {
        crate::agents::fmt_elapsed(agent.elapsed_secs)
    } else {
        format!("{} · {}", agent.status, crate::agents::fmt_elapsed(agent.elapsed_secs))
    };
    let progress = if !tools.is_empty() {
        let live = tools.iter().filter(|t| t.status == ToolStatus::Running).count();
        if live > 0 {
            format!("{} tool calls · {live} running", tools.len())
        } else {
            format!("{} tool calls", tools.len())
        }
    } else if agent.steps_total > 0 {
        format!("step {}/{}", agent.steps_done, agent.steps_total)
    } else {
        String::new()
    };
    div()
        .id(("agent-card", id))
        .test_support()
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
                .child(div().min_w_0().text_ellipsis().child(agent.name.clone()))
                .child(div().flex_1().flex_shrink_0())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(timing))
                .when(agent.status == AgentStatus::Running, |d| {
                    d.child(
                        div()
                            .id(("cancel-agent", id))
                            .test_support()
                            .cursor_pointer()
                            .text_color(cx.theme().muted_foreground)
                            .child(IconName::CircleX)
                            .on_click(cx.listener(move |this, _, _, cx| this.cancel_agent(id, cx))),
                    )
                })
                .child(
                    div()
                        .id(("expand-agent", id))
                        .test_support()
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(if agent.expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_agent_expand(id, cx))),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} · {}", agent.lane, agent.step)),
        )
        .when(!progress.is_empty(), |d| {
            d.child(div().text_xs().text_color(cx.theme().muted_foreground).child(progress))
        })
        .children(tools.into_iter().map(|tool| tool_row(agent, tool, cx)))
        .when(agent.expanded && !agent.log.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .mt_1()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().input)
                    .children(agent.log.iter().map(|line| div().text_xs().child(line.clone()))),
            )
        })
        .into_any_element()
}

/// One tool call under an agent card: status icon, name, detail, and a
/// chevron that expands the captured output — the same shape Codex uses
/// for tool work inside a turn.
fn tool_row(agent: &Agent, tool: &ToolCall, cx: &mut Context<Workspace>) -> AnyElement {
    // A tool still marked Running under a finished/cancelled turn was
    // interrupted — show it as failed rather than spinning forever.
    let status = if tool.status == ToolStatus::Running && agent.status != AgentStatus::Running {
        ToolStatus::Failed
    } else {
        tool.status
    };
    let (icon, color) = match status {
        ToolStatus::Running => (IconName::LoaderCircle, cx.theme().info),
        ToolStatus::Done => (IconName::CircleCheck, cx.theme().success),
        ToolStatus::Failed => (IconName::CircleX, cx.theme().danger),
    };
    let agent_id = agent.id;
    let ix = tool.tool_ix;
    let expanded = agent.expanded_tools.contains(&ix);
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .id(SharedString::from(format!("agent-tool-{agent_id}-{ix}")))
                .test_support()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .text_xs()
                .child(div().text_color(color).child(icon))
                .child(div().flex_shrink_0().child(tool.name.clone()))
                .child(
                    div()
                        .min_w_0()
                        .text_ellipsis()
                        .text_color(cx.theme().muted_foreground)
                        .child(tool.detail.clone()),
                )
                .child(div().flex_1().flex_shrink_0())
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_agent_tool(agent_id, ix, cx))),
        )
        .when(expanded && !tool.output.is_empty(), |d| {
            d.child(
                div()
                    .id(SharedString::from(format!("agent-tool-out-{agent_id}-{ix}")))
                    .test_support()
                    .mx_2()
                    .mb_1()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().input)
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(tool.output.clone()),
            )
        })
        .into_any_element()
}
