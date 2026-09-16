//! Snapshots-panel rendering: the header, one row per checkpoint across all
//! chats (age, chat title, changed-file count, size) with Restore/Delete
//! actions, and a footer with the retention policy menus. State and the
//! store ops live in `crate::snapshots`.
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::snapshots::SnapshotInfo;
use crate::workspace::Workspace;

/// Age options in the retention menu — `0` renders as "Forever".
const RETENTION_DAYS: [u32; 4] = [0, 7, 30, 90];
/// Size-cap options in the retention menu — `0` renders as "No cap".
const CAP_MB: [u32; 4] = [0, 256, 1024, 4096];

impl Workspace {
    pub fn render_snapshots_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self.snapshots.list.iter().enumerate().map(|(ix, s)| snapshot_row(ix, s, cx)).collect();
        let total: u64 = self.snapshots.list.iter().map(|s| s.bytes).sum();
        div()
            .id("snapshots-panel")
            .test_support()
            .w(px(360.))
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
                    .font_weight(FontWeight::BOLD)
                    .child(IconName::Camera)
                    .child("Snapshots")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("refresh-snapshots")
                            .test_support()
                            .cursor_pointer()
                            .text_color(cx.theme().muted_foreground)
                            .child(IconName::RefreshCcw)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_snapshots(cx))),
                    )
                    .child(
                        div()
                            .id("close-snapshots")
                            .test_support()
                            .cursor_pointer()
                            .child(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_snapshots_panel(cx))),
                    ),
            )
            .child(
                div()
                    .id("snapshots-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(rows.is_empty(), |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No snapshots yet — one is taken before each turn"),
                        )
                    })
                    .children(rows),
            )
            .child(footer(self.snapshots.list.len(), total, self.snapshots.retention_days, self.snapshots.cap_mb, cx))
    }
}

/// One snapshot row: relative age + chat title on top, changed/size detail
/// under it, Restore and Delete trailing. Clicking the row expands the
/// paths restore would touch — computed lazily, cached on the row.
fn snapshot_row(ix: usize, snap: &SnapshotInfo, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let changed = snap.changed.map_or("—".to_string(), |n| n.to_string());
    let mut entry = div().flex().flex_col().child(
        div()
            .id(("snapshot-row", ix))
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .hover(|d| d.bg(theme.muted))
            .child(div().flex_shrink_0().text_color(theme.muted_foreground).child(if snap.expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_shrink_0().text_color(theme.muted_foreground).child(IconName::GitCommitHorizontal))
                            .child(div().flex_shrink_0().text_xs().text_color(theme.muted_foreground).child(rel_time(snap.at)))
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(snap.chat_title.clone())),
                    )
                    .child(
                        div()
                            .pl_6()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{changed} changed · {}", fmt_bytes(snap.bytes))),
                    ),
            )
            .child(
                div()
                    .id(("snapshot-restore", ix))
                    .test_support()
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .child(IconName::Undo2)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Keep the click off the row — it must not toggle the list.
                        cx.stop_propagation();
                        this.restore_snapshot(ix, window, cx);
                    })),
            )
            .child(
                div()
                    .id(("snapshot-delete", ix))
                    .test_support()
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .child(IconName::Trash)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.delete_snapshot(ix, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.expand_snapshot(ix, cx))),
    );
    if snap.expanded {
        entry = entry.child(super::snapshot_files::snapshot_files(ix, snap, cx));
    }
    entry.into_any_element()
}

/// Totals plus the retention policy: two dropdown menus whose checked item
/// is the current setting.
fn footer(count: usize, total: u64, days: u32, cap_mb: u32, cx: &mut Context<Workspace>) -> impl IntoElement {
    let ws_days = cx.entity();
    let ws_cap = cx.entity();
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(format!("{count} snapshots · {}", fmt_bytes(total)))
        .child(div().flex_1())
        .child(
            Button::new("snapshot-retention")
                .ghost()
                .small()
                .label(format!("Keep: {}", retention_label(days)))
                .dropdown_menu(move |menu, _, _| {
                    RETENTION_DAYS.iter().fold(menu, |menu, &d| {
                        let ws = ws_days.clone();
                        menu.item(PopupMenuItem::new(retention_label(d)).checked(d == days).on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.set_snapshot_retention(d, cx));
                        }))
                    })
                }),
        )
        .child(
            Button::new("snapshot-cap")
                .ghost()
                .small()
                .label(format!("Cap: {}", cap_label(cap_mb)))
                .dropdown_menu(move |menu, _, _| {
                    CAP_MB.iter().fold(menu, |menu, &m| {
                        let ws = ws_cap.clone();
                        menu.item(PopupMenuItem::new(cap_label(m)).checked(m == cap_mb).on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.set_snapshot_cap(m, cx));
                        }))
                    })
                }),
        )
}

fn retention_label(days: u32) -> String {
    if days == 0 { "Forever".to_string() } else { format!("{days} days") }
}

fn cap_label(mb: u32) -> String {
    if mb == 0 {
        "No cap".to_string()
    } else if mb >= 1024 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{mb} MB")
    }
}

/// "2h ago"-style age for snapshot rows.
fn rel_time(at: std::time::SystemTime) -> SharedString {
    let secs = at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3_600 => format!("{}m ago", s / 60).into(),
        s if s < 86_400 => format!("{}h ago", s / 3_600).into(),
        s => format!("{}d ago", s / 86_400).into(),
    }
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{} KB", n >> 10)
    } else {
        format!("{n} B")
    }
}
