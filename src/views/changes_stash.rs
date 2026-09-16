//! The stash section of the Changes panel's git block: a message input plus
//! Stash button (`git stash push -u`, "WIP" when the box is empty) and one
//! row per stash entry (`stash@{n}`, message, age). A row's right-click menu
//! offers Pop / Apply / Drop. Ops live in `crate::changes_stash`; this file
//! only renders `Workspace::git`.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::StashEntry;
use crate::workspace::Workspace;

/// The section — always mounted by `git_block` so the Stash button is
/// reachable on a clean tree; the list only shows when entries exist.
pub fn stash_section(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let rows = ws.git.stashes.iter().enumerate().map(|(ix, s)| stash_row(ix, s, cx)).collect::<Vec<_>>();
    v_flex()
        .id("stash-section")
        .test_support()
        .gap_0p5()
        .child(stash_row_input(ws, cx))
        .when(!rows.is_empty(), |d| {
            d.child(
                h_flex()
                    .gap_1()
                    .pt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(IconName::Archive)
                    .child("Stashes"),
            )
            .child(v_flex().id("stash-list").max_h(px(160.)).overflow_y_scroll().gap_0p5().children(rows))
        })
        .into_any_element()
}

/// The stash-message input plus its Stash button — Enter in the input runs
/// the same `stash push`. The button is only refused while an op is running;
/// an empty message falls back to "WIP".
fn stash_row_input(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (border, muted_fg) = (theme.border, theme.muted_foreground);
    let busy = ws.git.busy;
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().flex_1().min_w_0().child(Input::new(&ws.git.stash_input).id("stash-input").xsmall().w_full()))
        .child(
            div()
                .id("stash-button")
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
                .when(!busy, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.stash_changes(cx))))
                .child(IconName::Archive)
                .child("Stash"),
        )
        .into_any_element()
}

/// `stash@{0} On main: msg … 2h ago` — right-click opens the stash menu.
fn stash_row(ix: usize, stash: &StashEntry, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let (muted_fg, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
    let name = stash.name.clone();
    div()
        .id(("stash-row", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .text_xs()
        .hover(|d| d.bg(cx.theme().muted))
        .child(div().flex_shrink_0().font_family(mono).text_color(cx.theme().info).child(stash.name.clone()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(stash.message.clone()),
        )
        .child(div().flex_shrink_0().text_color(muted_fg).child(stash.rel_time.clone()))
        .context_menu({
            let ws = cx.entity();
            move |menu, _window, cx| stash_menu(&ws, &name, menu, cx)
        })
        .into_any_element()
}

/// A stash row's right-click menu: pop (apply + drop), apply (keep the
/// entry), or drop it without applying.
fn stash_menu(ws: &Entity<Workspace>, name: &str, menu: PopupMenu, _cx: &mut Context<PopupMenu>) -> PopupMenu {
    let (ws_pop, ws_apply, ws_drop) = (ws.clone(), ws.clone(), ws.clone());
    let (pop, apply, drop) = (name.to_string(), name.to_string(), name.to_string());
    menu.item(PopupMenuItem::new("Pop").icon(IconName::ArchiveRestore).on_click(move |_, _, cx| {
        ws_pop.update(cx, |this, cx| this.stash_pop(&pop, cx));
    }))
    .item(PopupMenuItem::new("Apply").icon(IconName::Import).on_click(move |_, _, cx| {
        ws_apply.update(cx, |this, cx| this.stash_apply(&apply, cx));
    }))
    .item(PopupMenuItem::new("Drop").icon(IconName::Trash).on_click(move |_, _, cx| {
        ws_drop.update(cx, |this, cx| this.stash_drop(&drop, cx));
    }))
}
