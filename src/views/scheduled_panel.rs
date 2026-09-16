//! The Scheduled panel — a persistent right-side list of this project's
//! scheduled prompts (see `crate::automations`). Rows show the prompt
//! preview, its chat, interval and next run; the switch toggles `enabled`
//! and the trash button deletes. Clicking a row selects its chat.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::SystemTime;

use crate::automations::Automation;
use crate::workspace::Workspace;

impl Workspace {
    pub fn render_scheduled_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .automations
            .iter()
            .map(|a| {
                let chat = self.chats.iter().find(|c| c.id == a.chat_id);
                automation_row(a, chat, cx)
            })
            .collect();

        div()
            .id("scheduled-panel")
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
                    .child(IconName::CalendarClock)
                    .child("Scheduled")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("close-scheduled")
                            .test_support()
                            .cursor_pointer()
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_scheduled_panel(cx))),
                    ),
            )
            .child(
                div()
                    .id("scheduled-list")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py_1()
                    .flex()
                    .flex_col()
                    .when(rows.is_empty(), |d| {
                        d.child(
                            div()
                                .id("scheduled-empty")
                                .test_support()
                                .aria_label("No scheduled prompts")
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No scheduled prompts — use a chat's ⋯ menu to schedule one."),
                        )
                    })
                    .children(rows),
            )
    }
}

/// One automation row: prompt preview over a chat/interval/next-run line,
/// then the enable switch and delete button. Click selects the chat so
/// the transcript that ran the prompt is one tap away.
fn automation_row(a: &Automation, chat: Option<&crate::model::Chat>, cx: &mut Context<Workspace>) -> AnyElement {
    let id = a.id;
    let chat_id = a.chat_id;
    let preview = first_line(&a.prompt);
    let chat_title = chat.map(|c| c.title.clone()).unwrap_or_else(|| "Deleted chat".into());
    let meta = format!("{chat_title} · every {} · {}", a.interval.label(), next_run_label(a));
    div()
        .id(("scheduled-row", id))
        .test_support()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .hover(|d| d.bg(cx.theme().sidebar_accent.opacity(0.5)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_sm().overflow_hidden().text_ellipsis().whitespace_nowrap().child(preview))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(meta),
                ),
        )
        .child(
            Switch::new(SharedString::from(format!("scheduled-enable-{id}")))
                .checked(a.enabled)
                .small()
                .accessibility_label("Enable scheduled prompt")
                .on_click(cx.listener(move |this, on, _, cx| this.toggle_automation(id, *on, cx))),
        )
        .child(
            div()
                .id(("scheduled-delete", id))
                .test_support()
                .cursor_pointer()
                .text_color(cx.theme().muted_foreground)
                .child(IconName::Trash)
                .on_click(cx.listener(move |this, _, _, cx| this.delete_automation(id, cx))),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if let Some(ix) = this.chat_index(chat_id) {
                this.select_chat(ix, window, cx);
            }
        }))
        .into_any_element()
}

/// First line of the prompt, capped at 60 chars — the row's preview.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line.to_string()
    }
}

/// "next in 15m" / "next now" / "paused" — the row's schedule label.
fn next_run_label(a: &Automation) -> String {
    if !a.enabled {
        return "paused".to_string();
    }
    let secs = a.next_run.duration_since(SystemTime::now()).map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 {
        "next now".to_string()
    } else if secs < 3600 {
        format!("next in {}m", secs.div_ceil(60))
    } else if secs < 86400 {
        format!("next in {}h", secs.div_ceil(3600))
    } else {
        format!("next in {}d", secs.div_ceil(86400))
    }
}

/// The sidebar's Scheduled row — opens the panel; the suffix carries the
/// enabled-automation count so the schedule is glanceable while closed.
/// Extracted so `sidebar.rs` stays under the SLOC cap.
pub(crate) fn scheduled_nav_row(ws: &Workspace, cx: &mut Context<Workspace>) -> super::nav_row::NavRow {
    let count = ws.automations.iter().filter(|a| a.enabled).count();
    super::nav_row::NavRow::new("sidebar-scheduled", "Scheduled")
        .icon(IconName::CalendarClock)
        .active(ws.scheduled_panel_open)
        .suffix(move |_, cx| {
            div().id("sidebar-scheduled-count").test_support().text_xs().when(count > 0, |d| {
                d.aria_label(format!("{count} scheduled"))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{count}"))
            })
        })
        .on_click(cx.listener(|this, _, _, cx| this.toggle_scheduled_panel(cx)))
}
