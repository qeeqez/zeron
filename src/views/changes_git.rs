//! Git action block at the foot of the Changes panel: the branch header
//! (in `changes_git_branch`), the commit-message input with its ✦ generate
//! button and Commit button, Push and Create PR, the stash input + list,
//! and the last op's status note. Ops live in `crate::changes`,
//! `crate::changes_stash` and `crate::changes_generate`; this file only
//! renders `Workspace::git`.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
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
        .child(super::changes_git_branch::branch_row(ws, branch, cx))
        .child(commit_row(ws, cx))
        .child(action_row(ws, cx))
        .child(crate::views::changes_stash::stash_section(ws, cx))
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
        .when(!ws.git.commits.is_empty(), |d| d.child(crate::views::changes_commits::commits_section(ws, cx)))
        .into_any_element()
}

/// The commit-message input plus the ✦ generate and Commit buttons —
/// Commit is disabled while the message is empty or a git op is running;
/// ✦ is disabled while any op or generation is in flight.
fn commit_row(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (accent, accent_fg, muted, muted_fg) = (theme.accent, theme.accent_foreground, theme.muted, theme.muted_foreground);
    let ready = !ws.git.busy && !ws.git.commit_input.read(cx).value().trim().is_empty();
    let idle = !ws.git.busy && !ws.git.generating;
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
                .id("generate-message")
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .rounded_md()
                .px_1p5()
                .py_1()
                .text_xs()
                .text_color(muted_fg)
                .when(idle, |d| {
                    d.cursor_pointer()
                        .hover(|d| d.bg(muted))
                        .on_click(cx.listener(|this, _, _, cx| this.generate_commit_message(cx)))
                })
                .child(if ws.git.generating {
                    Spinner::new().xsmall().into_any_element()
                } else {
                    IconName::Sparkles.into_any_element()
                }),
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

#[cfg(test)]
#[path = "../changes_git_ui_tests.rs"]
mod changes_git_ui_tests;
