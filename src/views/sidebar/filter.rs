//! The sidebar's search area — the chat-list search field plus the filter
//! chips under it. One ghost button per `SidebarFilter`, accent-filled
//! while on; chips AND together with the title query (see
//! `crate::sidebar_filter`). Also owns the muted "No chats match"
//! placeholder a narrowing search falls back to.

use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::sidebar_filter::SidebarFilter;
use crate::workspace::Workspace;

impl Workspace {
    /// The chat-list search field — only mounted on the Chats tab.
    pub(super) fn search_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().flex_1().child(Input::new(&self.search).prefix(IconName::Search).appearance(true)))
            .when(!self.search.read(cx).value().is_empty(), |d| {
                d.child(
                    div()
                        .id("search-clear")
                        .test_support()
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::X)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search.update(cx, |s, cx| s.set_value("", window, cx));
                        })),
                )
            })
    }

    /// The toggle chips under the chat search. `shown`/`total` feed the
    /// "N of M" count that appears while any chip is on.
    pub(super) fn filter_row(&self, shown: usize, total: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().id("sidebar-filters").test_support().flex().items_center().gap_1();
        for f in SidebarFilter::ALL {
            let active = self.sidebar_filters.is_active(f);
            row = row.child(
                Button::new(f.id())
                    .ghost()
                    .xsmall()
                    .icon(f.icon())
                    .label(f.label())
                    .selected(active)
                    .toggled(active)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar_filters.toggle(f);
                        cx.notify();
                    })),
            );
        }
        row.when_some(self.sidebar_filters.count_text(shown, total), |d, count| {
            d.child(
                div()
                    .id("sidebar-filter-count")
                    .test_support()
                    .ml_auto()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(count),
            )
        })
    }
}

/// The muted placeholder a narrowing search renders when nothing matched —
/// same shape as the settings nav's "No settings match" row.
pub(super) fn no_match_group() -> super::group::ChatGroup {
    let row = crate::views::nav_row::NavRow::new("sidebar-no-match", "No chats match")
        .hoverable(false)
        .body(|_, cx| div().text_color(cx.theme().muted_foreground).child("No chats match"));
    super::group::ChatGroup::new("").child(row)
}
