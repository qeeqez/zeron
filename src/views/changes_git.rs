//! Git action block at the foot of the Changes panel: the branch header
//! (name, upstream, ahead/behind), the commit-message input + Commit button,
//! Push and Create PR, and the last op's status note. Ops live in
//! `crate::changes`; this file only renders `Workspace::git`.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::BranchStatus;
use crate::workspace::Workspace;

/// The whole block — mounted only when `Workspace::git.branch` is `Some`, so
/// non-repo projects never see it.
pub fn git_block(ws: &Workspace, branch: &BranchStatus, cx: &mut Context<Workspace>) -> AnyElement {
    div()
        .id("changes-git")
        .test_support()
        .flex()
        .flex_col()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(branch_row(branch, cx))
        .child(commit_row(ws, cx))
        .child(action_row(ws, cx))
        .when_some(ws.git.note.clone(), |d, (text, is_error)| {
            d.child(
                div()
                    .id("git-note")
                    .test_support()
                    .text_xs()
                    .text_color(if is_error { cx.theme().danger } else { cx.theme().muted_foreground })
                    .child(text),
            )
        })
        .into_any_element()
}

/// `⎇ branch ↑n ↓n → upstream` — the upstream name only shows when set.
fn branch_row(branch: &BranchStatus, cx: &mut Context<Workspace>) -> AnyElement {
    div()
        .id("git-branch")
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(IconName::GitBranch))
        .child(div().font_bold().child(branch.name.clone()))
        .when(branch.ahead > 0, |d| d.child(div().text_color(cx.theme().info).child(format!("↑{}", branch.ahead))))
        .when(branch.behind > 0, |d| d.child(div().text_color(cx.theme().warning).child(format!("↓{}", branch.behind))))
        .child(div().flex_1())
        .when_some(branch.upstream.clone(), |d, up| d.child(div().text_color(cx.theme().muted_foreground).child(up)))
        .into_any_element()
}

/// The commit-message input plus its Commit button — disabled while the
/// message is empty or another op is running.
fn commit_row(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (accent, accent_fg, muted_fg) = (theme.accent, theme.accent_foreground, theme.muted_foreground);
    let ready = !ws.git.busy && !ws.git.commit_input.read(cx).value().trim().is_empty();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&ws.git.commit_input).id("commit-input").xsmall().w_full()),
        )
        .child(
            div()
                .id("commit-button")
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .rounded_md()
                .px_2()
                .py_1()
                .text_xs()
                .when(ready, |d| {
                    d.cursor_pointer()
                        .bg(accent)
                        .text_color(accent_fg)
                        .on_click(cx.listener(|this, _, _, cx| this.commit_staged(cx)))
                })
                .when(!ready, |d| d.text_color(muted_fg))
                .child(IconName::Check)
                .child("Commit"),
        )
        .into_any_element()
}

/// Push and Create PR — both refused while an op is in flight.
fn action_row(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (border, muted_fg) = (theme.border, theme.muted_foreground);
    let busy = ws.git.busy;
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id("push-button")
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .rounded_md()
                .px_2()
                .py_1()
                .text_xs()
                .border_1()
                .border_color(border)
                .text_color(muted_fg)
                .when(!busy, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.push_changes(cx))))
                .child(IconName::Upload)
                .child("Push"),
        )
        .child(
            div()
                .id("create-pr-button")
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .rounded_md()
                .px_2()
                .py_1()
                .text_xs()
                .border_1()
                .border_color(border)
                .text_color(muted_fg)
                .when(!busy, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.create_pr(cx))))
                .child(IconName::GitPullRequestCreate)
                .child("Create PR"),
        )
        .into_any_element()
}
