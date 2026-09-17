//! The "Messages" sidebar group — message-body hits for the chat-list
//! query, under the chat groups. One row per chat: title, the match
//! snippet with the query highlighted, and the chat's match count. Rows
//! past `sidebar_search::MAX_ROWS` collapse into a "+N more" footer that
//! opens the full search dialog with the query carried over.

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::global_search::sidebar_search::SidebarMsgHit;
use crate::views::nav_row::NavRow;
use crate::workspace::Workspace;

use super::group::ChatGroup;

impl Workspace {
    /// The "Messages" group for the current query — `None` while the query
    /// is empty or nothing matched, so the group never renders a bare
    /// header.
    pub(super) fn messages_group(&self, query: &str, cx: &mut Context<Self>) -> Option<ChatGroup> {
        if query.is_empty() || self.sidebar_hits.is_empty() {
            return None;
        }
        let mut group = ChatGroup::new("Messages").children(self.sidebar_hits.iter().map(|h| hit_row(h, query, cx)));
        if self.sidebar_hits_extra > 0 {
            let query: SharedString = query.to_string().into();
            group = group.child(
                NavRow::new("sidebar-hits-more", format!("+{} more", self.sidebar_hits_extra))
                    .icon(IconName::TextSearch)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_global_search_seeded(&query, window, cx);
                    })),
            );
        }
        Some(group)
    }
}

/// One hit row: chat title, then the match snippet (query highlighted),
/// then the chat's match count. Click opens the chat at the message.
fn hit_row(hit: &SidebarMsgHit, query: &str, cx: &mut Context<Workspace>) -> NavRow {
    let file_ix = hit.file_ix;
    let title = hit.title.clone();
    let snippet = hit.snippet.clone();
    let count = hit.count;
    let highlight = query.to_lowercase();
    let hit_click = hit.clone();
    let query_click: SharedString = query.to_string().into();
    NavRow::new(("sidebar-hit", file_ix), "")
        .icon(IconName::MessageSquare)
        .body(move |_, cx| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .items_center()
                .child(div().flex_shrink_0().max_w_1_2().overflow_x_hidden().whitespace_nowrap().child(title.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(highlighted(&snippet, &highlight, cx)),
                )
                .child(
                    div()
                        .id(("sidebar-hit-count", file_ix))
                        .test_support()
                        .aria_label(count.to_string())
                        .flex_shrink_0()
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .bg(cx.theme().muted)
                        .child(count.to_string()),
                )
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_hit(&hit_click.as_search_hit(), &query_click, window, cx);
        }))
}

/// The snippet with the query's first occurrence accent-highlighted —
/// `StyledText` takes byte ranges, so the match is re-found on the
/// lowercase form and snapped to char boundaries (lowercasing can shift
/// offsets).
fn highlighted(snippet: &str, query: &str, cx: &App) -> StyledText {
    let range = snippet.to_lowercase().find(query).and_then(|start| {
        let mut start = start.min(snippet.len());
        while start > 0 && !snippet.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (start + query.len()).min(snippet.len());
        while end < snippet.len() && !snippet.is_char_boundary(end) {
            end += 1;
        }
        (start < end).then_some(start..end)
    });
    match range {
        Some(range) => StyledText::new(snippet.to_string()).with_highlights([(
            range,
            HighlightStyle {
                color: Some(cx.theme().accent),
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
        )]),
        None => StyledText::new(snippet.to_string()),
    }
}
