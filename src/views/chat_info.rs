//! The chat info dialog — a read-only details card for the active chat,
//! opened from the ⋯ menu's "Chat info" item. Everything it shows is
//! already on `Chat`/`Workspace` (created date, message count, stamped
//! provider/model/access, folded token usage, worktree path, ephemeral
//! flag, resumed thread id) — this is a pure view; the rows snapshot at
//! open because the dialog builder runs during Root's render, where the
//! workspace is already borrowed.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::pricing::fmt_cost;
use crate::workspace::Workspace;

impl Workspace {
    /// The ⋯ menu's "Chat info" — a dialog titled with the chat title
    /// listing its details. Read-only: Esc, the backdrop and the × all
    /// close it; there are no actions.
    pub fn open_chat_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.chats[self.active].title.clone();
        let rows = info_rows(self, &self.chats[self.active]);
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .title(div().id("chat-info-title").test_support().aria_label(title.to_string()).child(title.clone()))
                .overlay_closable(true)
                .child(
                    v_flex()
                        .id("chat-info")
                        .test_support()
                        .gap_1()
                        .text_sm()
                        .children(rows.iter().map(|row| info_row(row, cx))),
                )
        });
    }
}

/// One definition-list row's data — bundled so `info_row` stays under the
/// arg-count lint. `full` carries the untruncated value for the tooltip
/// when the visible text can clip (paths, thread ids).
struct InfoRow {
    id: &'static str,
    label: &'static str,
    value: String,
    full: Option<String>,
}

/// The dialog's rows: the always-present facts first, then the optional
/// ones — a non-worktree chat has no Worktree row, a fresh thread no
/// Thread id row.
fn info_rows(this: &Workspace, chat: &Chat) -> Vec<InfoRow> {
    // Legacy chats carry no provider/model stamp — they follow the live
    // workspace selection, same fallback as the usage popover.
    let model_id = if chat.model.is_empty() { this.model.as_ref() } else { chat.model.as_str() };
    let provider_id = if chat.provider.is_empty() { this.selected_provider.as_str() } else { chat.provider.as_str() };
    let model = this
        .model_catalog
        .get(provider_id)
        .and_then(|ms| ms.iter().find(|m| m.id.as_ref() == model_id))
        .map_or_else(|| model_id.to_string(), |m| m.label.to_string());
    let provider = this
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .map_or_else(|| provider_id.to_string(), |p| p.name.clone());
    let access = chat.access.unwrap_or(this.access);
    let tokens = chat.usage.tokens();
    // Honest cost: a priced model shows the estimate, anything else '—'.
    let cost = chat.usage.cost(model_id).map(|c| format!("~{}", fmt_cost(c))).unwrap_or_else(|| "—".into());

    let mut rows = vec![
        InfoRow {
            id: "chat-info-created",
            label: "Created",
            value: created_label(chat.created_at),
            full: None,
        },
        InfoRow {
            id: "chat-info-messages",
            label: "Messages",
            value: chat.messages.len().to_string(),
            full: None,
        },
        InfoRow {
            id: "chat-info-model",
            label: "Model",
            value: model,
            full: None,
        },
        InfoRow {
            id: "chat-info-provider",
            label: "Provider",
            value: provider,
            full: None,
        },
        InfoRow {
            id: "chat-info-access",
            label: "Access mode",
            value: access.label().to_string(),
            full: None,
        },
        InfoRow {
            id: "chat-info-tokens",
            label: "Tokens",
            value: super::usage_popover::token_split(tokens),
            full: None,
        },
        InfoRow {
            id: "chat-info-cost",
            label: "Est. cost",
            value: cost,
            full: None,
        },
    ];
    if chat.worktree && !chat.workdir.is_empty() {
        rows.push(InfoRow {
            id: "chat-info-worktree",
            label: "Worktree",
            value: chat.workdir.clone(),
            full: Some(chat.workdir.clone()),
        });
    }
    rows.push(InfoRow {
        id: "chat-info-temporary",
        label: "Temporary",
        value: if chat.ephemeral { "Yes" } else { "No" }.to_string(),
        full: None,
    });
    if !chat.thread_id.is_empty() {
        rows.push(InfoRow {
            id: "chat-info-thread",
            label: "Thread id",
            value: chat.thread_id.clone(),
            full: Some(chat.thread_id.clone()),
        });
    }
    rows
}

/// One label/value row — the aria label carries the full value so tests
/// and screen readers see past any ellipsis.
fn info_row(row: &InfoRow, cx: &App) -> impl IntoElement {
    let mut value = div()
        .id((row.id, 1usize))
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_right()
        .child(row.value.clone());
    if let Some(full) = row.full.clone() {
        value = value.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
    }
    h_flex()
        .id(row.id)
        .test_support()
        .aria_label(format!("{}: {}", row.label, row.value))
        .justify_between()
        .gap_2()
        .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(row.label))
        .child(value)
}

/// "Mar 3, 2026, 4:05 PM" — the message footer's local-time conversion
/// with the date spelled out.
fn created_label(at: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at).format("%b %-d, %Y, %-I:%M %p").to_string()
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "../chat_info_tests.rs"]
mod chat_info_tests;
