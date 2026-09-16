//! The Changes panel's branch header: the branch-name trigger, its picker
//! dropdown (one row per local branch plus a new-branch input), and the
//! ahead/behind + upstream readout. Split from `changes_git` to stay under
//! the SLOC cap; ops live in `crate::changes`.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{Branch, BranchStatus};
use crate::workspace::Workspace;

/// `⎇ branch ↑n ↓n → upstream` — the name is a button opening the branch
/// picker; the upstream name only shows when set. Right-click opens the
/// file menu for the repo root (empty path).
pub(super) fn branch_row(ws: &Workspace, branch: &BranchStatus, cx: &mut Context<Workspace>) -> AnyElement {
    let spec = BranchPickerSpec {
        ws: cx.entity(),
        current: branch.name.clone(),
        branches: ws.git.branches.clone(),
        new_branch_input: ws.git.new_branch_input.clone(),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(branch_picker(spec))
        .when(branch.ahead > 0, |d| d.child(div().text_color(cx.theme().info).child(format!("↑{}", branch.ahead))))
        .when(branch.behind > 0, |d| d.child(div().text_color(cx.theme().warning).child(format!("↓{}", branch.behind))))
        .child(div().flex_1())
        .when_some(branch.upstream.clone(), |d, up| d.child(div().text_color(cx.theme().muted_foreground).child(up)))
        .context_menu({
            let ws = cx.entity();
            move |menu, window, cx| crate::open_in::file_menu(&ws, "", menu, window, cx)
        })
        .into_any_element()
}

/// Owned inputs for `branch_picker` — `git_block` builds this from
/// `&Workspace` so the popover's `'static` content closure holds no borrow.
struct BranchPickerSpec {
    ws: Entity<Workspace>,
    /// Current branch name — the check mark's target.
    current: String,
    /// Local branches from the last `refresh_branches`.
    branches: Vec<Branch>,
    /// The new-branch name input — lives on `ChangesGit` so it survives the
    /// popover closing.
    new_branch_input: Entity<InputState>,
}

/// The branch-name trigger plus its dropdown: one row per local branch and
/// a new-branch input at the foot. Opening re-lists branches so ones made
/// outside the app show up.
fn branch_picker(spec: BranchPickerSpec) -> impl IntoElement {
    let ws = spec.ws.clone();
    Popover::new("branch-picker")
        .trigger(
            Button::new("git-branch")
                .ghost()
                .xsmall()
                .label(spec.current.clone())
                .icon(IconName::GitBranch)
                .dropdown_caret(true),
        )
        .on_open_change(move |open, _, cx| {
            if *open {
                ws.update(cx, |this, cx| this.refresh_branches(cx));
            }
        })
        .content(move |_, _, cx| picker_body(&spec, cx.entity(), cx))
}

/// The dropdown's single column: branch rows, then the new-branch input.
fn picker_body(spec: &BranchPickerSpec, popover: Entity<PopoverState>, cx: &App) -> AnyElement {
    let rows = spec
        .branches
        .iter()
        .map(|b| branch_opt(b, b.name == spec.current, &spec.ws, &popover, cx))
        .collect::<Vec<_>>();
    v_flex()
        .id("branch-picker-body")
        .test_support()
        .w(px(220.))
        .gap_0p5()
        .when(rows.is_empty(), |d| {
            d.child(div().px_2().py_1().text_sm().text_color(cx.theme().muted_foreground).child("No local branches"))
        })
        .child(v_flex().id("branch-picker-list").max_h(px(280.)).overflow_y_scroll().gap_0p5().children(rows))
        .child(new_branch_row(spec, cx))
        .into_any_element()
}

/// One branch row: name, a check on the checked-out one, click switches and
/// dismisses the popover.
fn branch_opt(b: &Branch, current: bool, ws: &Entity<Workspace>, popover: &Entity<PopoverState>, cx: &App) -> impl IntoElement {
    let name = b.name.clone();
    let ws = ws.clone();
    let popover = popover.clone();
    h_flex()
        .id(SharedString::from(format!("branch-opt-{}", b.name)))
        .test_support()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .hover(|d| d.bg(cx.theme().accent))
        .child(b.name.clone())
        .child(div().flex_1())
        .when(current, |d| d.child(IconName::Check))
        .on_click(move |_, window, cx| {
            ws.update(cx, |this, cx| this.checkout_branch(&name, cx));
            popover.update(cx, |state, cx| state.dismiss(window, cx));
        })
}

/// The new-branch input plus its create button — Enter in the input runs
/// the same `create_branch` op. Disabled while the name is empty or an op
/// is running.
fn new_branch_row(spec: &BranchPickerSpec, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (accent, accent_fg, muted_fg) = (theme.accent, theme.accent_foreground, theme.muted_foreground);
    let ready = !spec.ws.read(cx).git.busy && !spec.new_branch_input.read(cx).value().trim().is_empty();
    let ws = spec.ws.clone();
    h_flex()
        .items_center()
        .gap_1()
        .pt_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&spec.new_branch_input).id("new-branch-input").xsmall().w_full()),
        )
        .child(
            div()
                .id("new-branch-button")
                .test_support()
                .flex()
                .items_center()
                .rounded_md()
                .px_2()
                .py_1()
                .text_xs()
                .when(ready, |d| {
                    d.cursor_pointer().bg(accent).text_color(accent_fg).on_click(move |_, _, cx| {
                        ws.update(cx, |this, cx| this.create_branch(cx));
                    })
                })
                .when(!ready, |d| d.text_color(muted_fg))
                .child(IconName::Plus),
        )
}
