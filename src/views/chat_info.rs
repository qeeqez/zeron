//! The chat info popover — a read-only details card for the active chat,
//! anchored on the titlebar title (also reachable from the ⋯ menu's "Chat
//! info"). Everything it shows is already on `Chat`/`Workspace` (created
//! date, message count, stamped provider/model/access, folded token usage,
//! worktree path, ephemeral flag, resumed thread id) — this is a pure
//! view. The thread id row copies to the clipboard on click; the worktree
//! row reveals its checkout in Finder. Esc and click-away close it.

use gpui_kit::assets::IconName;
use gpui_kit::base::Popover;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ThemeStyled, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::pricing::fmt_cost;
use crate::workspace::Workspace;

impl Workspace {
    /// Open the titlebar's chat-info popover — the ⋯ menu's "Chat info"
    /// entry point. The popover is controlled by `chat_info_open` so this
    /// works outside render, where the popover's keyed state can't be
    /// reached; `on_open_change` folds user dismissals back into the flag.
    pub fn open_chat_info(&mut self, cx: &mut Context<Self>) {
        self.chat_info_open = true;
        cx.notify();
    }
}

/// The titlebar title wrapped as the popover's trigger: click opens the
/// details card under the title. `open` is the workspace flag — the ⋯
/// menu's "Chat info" sets it, `on_open_change` writes user toggles back.
pub fn chat_info_popover(title: SharedString, open: bool, ws: &Entity<Workspace>) -> impl IntoElement {
    let ws_flag = ws.clone();
    let ws_body = ws.clone();
    Popover::new("chat-info-popover")
        .anchor(Anchor::TopLeft)
        .open(open)
        .on_open_change(move |open, _, cx| {
            ws_flag.update(cx, |this, _| this.chat_info_open = *open);
        })
        .trigger_with(move |_, _, cx| {
            h_flex()
                .id("chat-title")
                .test_support()
                .gap_1()
                .px_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|d| d.bg(cx.theme().accent))
                .child(title)
                .child(div().text_color(cx.theme().muted_foreground).child(IconName::Info))
                .into_any_element()
        })
        .content(move |_, _, cx| chat_info_body(&ws_body, cx))
}

/// The popover's styled surface: the chat title, then the detail rows.
/// Reading `ws` here is safe — popover content draws in the deferred pass,
/// after the workspace's own render borrow is released.
fn chat_info_body(ws: &Entity<Workspace>, cx: &mut Context<gpui_kit::base::PopoverState>) -> AnyElement {
    let ws_entity = ws.clone();
    let ws = ws.read(cx);
    let chat = &ws.chats[ws.active];
    let rows = info_rows(ws, chat);
    v_flex()
        .id("chat-info")
        .test_support()
        .gap_1()
        .w(px(300.))
        .text_sm()
        .child(
            div()
                .id("chat-info-title")
                .test_support()
                .aria_label(chat.title.to_string())
                .font_weight(FontWeight::SEMIBOLD)
                .pb_1()
                .child(chat.title.clone()),
        )
        .children(rows.iter().map(|row| info_row(row, &ws_entity, cx)))
        .popover_style(cx)
        .p_3()
        .top_1()
        .into_any_element()
}

/// One definition-list row's data — bundled so `info_row` stays under the
/// arg-count lint. `full` carries the untruncated value for the tooltip
/// when the visible text can clip (paths, thread ids); `action` marks the
/// row clickable — copy the full value, or reveal it in Finder.
struct InfoRow {
    id: &'static str,
    label: &'static str,
    value: String,
    full: Option<String>,
    action: Option<RowAction>,
}

/// What a row click does with the row's `full` value.
#[derive(Clone, Copy)]
enum RowAction {
    /// Write the value to the clipboard (thread id).
    Copy,
    /// `open -R` the value as an absolute path (worktree checkout).
    Reveal,
}

/// The popover's rows: the always-present facts first, then the optional
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
            action: None,
        },
        InfoRow {
            id: "chat-info-messages",
            label: "Messages",
            value: chat.messages.len().to_string(),
            full: None,
            action: None,
        },
        InfoRow {
            id: "chat-info-model",
            label: "Model",
            value: model,
            full: None,
            action: None,
        },
        InfoRow {
            id: "chat-info-provider",
            label: "Provider",
            value: provider,
            full: None,
            action: None,
        },
        InfoRow {
            id: "chat-info-access",
            label: "Access mode",
            value: access.label().to_string(),
            full: None,
            action: None,
        },
        InfoRow {
            id: "chat-info-tokens",
            label: "Tokens",
            value: super::usage_popover::token_split(tokens),
            full: None,
            action: None,
        },
        InfoRow {
            id: "chat-info-cost",
            label: "Est. cost",
            value: cost,
            full: None,
            action: None,
        },
    ];
    if chat.worktree && !chat.workdir.is_empty() {
        rows.push(InfoRow {
            id: "chat-info-worktree",
            label: "Worktree",
            value: chat.workdir.clone(),
            full: Some(chat.workdir.clone()),
            action: Some(RowAction::Reveal),
        });
    }
    rows.push(InfoRow {
        id: "chat-info-temporary",
        label: "Temporary",
        value: if chat.ephemeral { "Yes" } else { "No" }.to_string(),
        full: None,
        action: None,
    });
    if !chat.thread_id.is_empty() {
        rows.push(InfoRow {
            id: "chat-info-thread",
            label: "Thread id",
            value: chat.thread_id.clone(),
            full: Some(chat.thread_id.clone()),
            action: Some(RowAction::Copy),
        });
    }
    rows
}

/// One label/value row — the aria label carries the full value so tests
/// and screen readers see past any ellipsis. Actionable rows get a pointer
/// cursor, a hover tint and a trailing affordance icon.
fn info_row(row: &InfoRow, ws: &Entity<Workspace>, cx: &App) -> impl IntoElement {
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
    let mut el = h_flex()
        .id(row.id)
        .test_support()
        .aria_label(format!("{}: {}", row.label, row.value))
        .justify_between()
        .gap_2()
        .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(row.label))
        .child(value);
    match row.action {
        Some(RowAction::Copy) => {
            let text = row.full.clone().unwrap_or_else(|| row.value.clone());
            el = el
                .cursor_pointer()
                .hover(|d| d.text_color(cx.theme().foreground))
                .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(IconName::Copy))
                .on_click(move |_, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    window.push_notification(Notification::success("Thread id copied to clipboard"), cx);
                });
        },
        Some(RowAction::Reveal) => {
            let path = std::path::PathBuf::from(row.full.clone().unwrap_or_else(|| row.value.clone()));
            let ws = ws.clone();
            el = el
                .cursor_pointer()
                .hover(|d| d.text_color(cx.theme().foreground))
                .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(IconName::FolderOpen))
                .on_click(move |_, _, cx| {
                    ws.update(cx, |this, cx| this.reveal_path_in_finder(&path, cx));
                });
        },
        None => {},
    }
    el
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
