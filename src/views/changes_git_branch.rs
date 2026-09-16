//! The Changes panel's branch header: the branch-name trigger, its picker
//! dropdown (one row per local branch plus a new-branch input), and the
//! ahead/behind + upstream readout. Split from `changes_git` to stay under
//! the SLOC cap; ops live in `crate::changes`.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{Branch, BranchStatus};
use crate::workspace::Workspace;

/// `⎇ branch ↑n ↓n → upstream` — the name is a button opening the branch
/// picker; the upstream name only shows when set. Fetch and pull buttons sit
/// beside the ahead/behind badges, disabled while an op runs. While a rename
/// is armed the header swaps to the rename input. Right-click opens the file
/// menu for the repo root (empty path).
pub(super) fn branch_row(ws: &Workspace, branch: &BranchStatus, cx: &mut Context<Workspace>) -> AnyElement {
    if let Some(old) = ws.git.rename_target.clone() {
        return rename_row(ws, &old, cx);
    }
    let spec = BranchPickerSpec {
        ws: cx.entity(),
        current: branch.name.clone(),
        branches: ws.git.branches.clone(),
        new_branch_input: ws.git.new_branch_input.clone(),
    };
    let busy = ws.git.busy;
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(branch_picker(spec))
        .when(branch.ahead > 0, |d| d.child(div().text_color(cx.theme().info).child(format!("↑{}", branch.ahead))))
        .when(branch.behind > 0, |d| d.child(div().text_color(cx.theme().warning).child(format!("↓{}", branch.behind))))
        .child(fetch_button(busy, cx))
        .child(pull_button(busy, cx))
        .child(div().flex_1())
        .when_some(branch.upstream.clone(), |d, up| d.child(div().text_color(cx.theme().muted_foreground).child(up)))
        .context_menu({
            let ws = cx.entity();
            move |menu, window, cx| crate::open_in::file_menu(&ws, "", menu, window, cx)
        })
        .into_any_element()
}

/// The header's fetch button — `git fetch --prune`; disabled while an op
/// runs so ops can't interleave.
fn fetch_button(busy: bool, cx: &mut Context<Workspace>) -> Button {
    Button::new("fetch-button")
        .ghost()
        .xsmall()
        .icon(IconName::RefreshCw)
        .tooltip("Fetch")
        .disabled(busy)
        .on_click(cx.listener(|this, _, _, cx| this.fetch_remote(cx)))
}

/// The header's pull button — `git pull --ff-only`; disabled while an op
/// runs so ops can't interleave.
fn pull_button(busy: bool, cx: &mut Context<Workspace>) -> Button {
    Button::new("pull-button")
        .ghost()
        .xsmall()
        .icon(IconName::Download)
        .tooltip("Pull (fast-forward only)")
        .disabled(busy)
        .on_click(cx.listener(|this, _, _, cx| this.pull_remote(cx)))
}

/// The header while a rename is armed: `old →` plus the rename input, a ✓
/// that runs `git branch -m`, and a ✕ that disarms. Enter in the input
/// confirms too — the subscription lives on `ChangesGit`.
fn rename_row(ws: &Workspace, old: &str, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (accent, accent_fg, muted_fg) = (theme.accent, theme.accent_foreground, theme.muted_foreground);
    let ready = !ws.git.busy && !ws.git.rename_input.read(cx).value().trim().is_empty();
    h_flex()
        .items_center()
        .gap_1()
        .text_xs()
        .child(div().flex_shrink_0().text_color(muted_fg).child(format!("{old} →")))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&ws.git.rename_input).id("rename-branch-input").xsmall().w_full()),
        )
        .child(
            div()
                .id("rename-branch-confirm")
                .test_support()
                .flex()
                .items_center()
                .rounded_md()
                .px_2()
                .py_1()
                .when(ready, |d| {
                    d.cursor_pointer().bg(accent).text_color(accent_fg).on_click(cx.listener(|this, _, _, cx| {
                        this.rename_branch(cx);
                    }))
                })
                .when(!ready, |d| d.text_color(muted_fg))
                .child(IconName::Check),
        )
        .child(
            div()
                .id("rename-branch-cancel")
                .test_support()
                .flex()
                .items_center()
                .rounded_md()
                .px_1()
                .py_1()
                .cursor_pointer()
                .text_color(muted_fg)
                .on_click(cx.listener(|this, _, window, cx| this.cancel_rename_branch(window, cx)))
                .child(IconName::X),
        )
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
/// dismisses the popover. Right-click offers Rename and Delete — Delete is
/// never offered on the current branch (git would refuse anyway).
fn branch_opt(b: &Branch, current: bool, ws: &Entity<Workspace>, popover: &Entity<PopoverState>, cx: &App) -> impl IntoElement {
    let name = b.name.clone();
    let ws = ws.clone();
    let ws_menu = ws.clone();
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
        .context_menu({
            let name = b.name.clone();
            move |menu, _window, cx| branch_menu(&ws_menu, &name, current, menu, cx)
        })
}

/// A branch row's right-click menu: "Rename…" arms the header's rename
/// input (the popover dismisses on the menu click, so the input can't live
/// inside it); "Delete" runs a confirmed `git branch -d` — hidden on the
/// checked-out branch.
fn branch_menu(ws: &Entity<Workspace>, name: &str, current: bool, menu: PopupMenu, _cx: &mut Context<PopupMenu>) -> PopupMenu {
    let (ws_rename, ws_delete) = (ws.clone(), ws.clone());
    let (rename, delete) = (name.to_string(), name.to_string());
    menu.item(PopupMenuItem::new("Rename…").icon(IconName::Pencil).on_click(move |_, window, cx| {
        ws_rename.update(cx, |this, cx| this.begin_rename_branch(&rename, window, cx));
    }))
    .when(!current, |m| {
        m.item(PopupMenuItem::new("Delete").icon(IconName::Trash).on_click(move |_, window, cx| {
            ws_delete.update(cx, |this, cx| this.delete_branch(&delete, window, cx));
        }))
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
