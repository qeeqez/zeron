//! The Changes panel's PR row: `#N · state · ✓k ✗k …k` for the current
//! branch's pull request, plus a refresh affordance and an Open button that
//! hands the URL to `cx.open_url`. Mounted by `git_block` only when
//! `Workspace::git.pr` is `Some` — no `gh`, no repo, or no PR all hide it.
//! Status is fetched by `refresh_changes` / `Workspace::refresh_pr`.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{PrState, PrStatus};
use crate::workspace::Workspace;

/// `#N · Open · ✓2 ✗1 …3` — the state colors match the host's convention
/// (green open, accent merged, red closed); check counts only show the
/// groups that ran. The refresh icon re-fetches just the PR status; Open
/// launches `pr.url` in the browser.
pub fn pr_row(pr: &PrStatus, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (border, muted_fg) = (theme.border, theme.muted_foreground);
    let state_color = match pr.state {
        PrState::Open => theme.success,
        PrState::Merged => theme.accent,
        PrState::Closed => theme.danger,
    };
    let url = pr.url.clone();
    div()
        .id("pr-row")
        .test_support()
        .aria_label(format!("#{} {}", pr.number, pr.state.label()))
        .flex()
        .items_center()
        .gap_1p5()
        .text_xs()
        .child(div().flex_shrink_0().text_color(muted_fg).child(IconName::GitPullRequest))
        .child(div().flex_shrink_0().child(format!("#{}", pr.number)))
        .child(div().flex_shrink_0().text_color(muted_fg).child("·"))
        .child(div().flex_shrink_0().text_color(state_color).child(pr.state.label()))
        .when(pr.checks.pass > 0, |d| {
            d.child(div().flex_shrink_0().text_color(muted_fg).child("·"))
                .child(div().flex_shrink_0().text_color(theme.success).child(format!("✓{}", pr.checks.pass)))
        })
        .when(pr.checks.fail > 0, |d| {
            d.child(div().flex_shrink_0().text_color(muted_fg).child("·"))
                .child(div().flex_shrink_0().text_color(theme.danger).child(format!("✗{}", pr.checks.fail)))
        })
        .when(pr.checks.pending > 0, |d| {
            d.child(div().flex_shrink_0().text_color(muted_fg).child("·"))
                .child(div().flex_shrink_0().text_color(theme.warning).child(format!("…{}", pr.checks.pending)))
        })
        .child(div().flex_1())
        .child(
            div()
                .id("pr-refresh")
                .test_support()
                .flex()
                .items_center()
                .rounded_md()
                .px_1()
                .py_0p5()
                .text_color(muted_fg)
                .cursor_pointer()
                .hover(|d| d.bg(theme.muted))
                .on_click(cx.listener(|this, _, _, cx| this.refresh_pr(cx)))
                .child(IconName::RefreshCcw),
        )
        .child(
            div()
                .id("pr-open")
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .rounded_md()
                .px_2()
                .py_0p5()
                .text_xs()
                .border_1()
                .border_color(border)
                .text_color(muted_fg)
                .cursor_pointer()
                .hover(|d| d.bg(theme.muted))
                .on_click(move |_, _, cx| cx.open_url(&url))
                .child(IconName::ExternalLink)
                .child("Open"),
        )
        .into_any_element()
}

#[cfg(test)]
#[path = "../changes_pr_ui_tests.rs"]
mod changes_pr_ui_tests;
