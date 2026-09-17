//! The Changes panel's diff-base picker for worktree chats: a "Base: <ref>"
//! row under the panel title whose trigger opens a dropdown of local
//! branches and tags plus the default (the project HEAD's merge-base).
//! Picking a ref pins it on the chat (`Chat::diff_base`) and recomputes the
//! file list; a stale pick falls back to the default and says so. Ops live
//! in `crate::changes::base`.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::BaseRef;
use crate::workspace::Workspace;

/// `Base: <ref> ▾` plus, while a stale pick fell back, a muted note naming
/// the missing ref. Mounted only when the changes scope carries a base —
/// i.e. the active chat's worktree exists and its base resolved.
pub(crate) fn base_row(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(base) = ws.changes_scope().base.clone() else {
        return div().into_any_element();
    };
    let picked = ws.chats[ws.active].diff_base.clone();
    let note = base
        .stale
        .then(|| format!("“{}” is gone — showing {}", picked.as_deref().unwrap_or_default(), base.label));
    let spec = BasePickerSpec {
        ws: cx.entity(),
        label: base.label,
        default: ws.git.branch.as_ref().map_or_else(|| "HEAD".to_string(), |b| b.name.clone()),
        picked,
        stale: base.stale,
        refs: ws.git.diff_bases.clone(),
    };
    h_flex()
        .id("changes-base-row")
        .test_support()
        .items_center()
        .gap_2()
        .px_3()
        .pb_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child("Base:")
        .child(base_picker(spec))
        .when_some(note, |d, n| d.child(div().id("changes-base-note").test_support().child(n)))
        .into_any_element()
}

/// Owned inputs for `base_picker` — `base_row` builds this from `&Workspace`
/// so the popover's `'static` content closure holds no borrow.
struct BasePickerSpec {
    ws: Entity<Workspace>,
    /// The effective base's label — the trigger's text.
    label: String,
    /// The project HEAD branch's name — the default row's label.
    default: String,
    /// The chat's pinned ref — `None` follows the default.
    picked: Option<String>,
    /// The pinned ref no longer resolves — the default is in effect.
    stale: bool,
    /// Branches + tags from the last `refresh_diff_bases`.
    refs: Vec<BaseRef>,
}

/// The base trigger plus its dropdown: the default row, then one row per
/// branch and tag. Opening re-lists refs so ones made outside the app show
/// up.
fn base_picker(spec: BasePickerSpec) -> impl IntoElement {
    let ws = spec.ws.clone();
    Popover::new("diff-base-picker")
        .trigger(
            Button::new("diff-base")
                .ghost()
                .xsmall()
                .label(spec.label.clone())
                .icon(IconName::GitBranch)
                .dropdown_caret(true),
        )
        .on_open_change(move |open, _, cx| {
            if *open {
                ws.update(cx, |this, cx| this.refresh_diff_bases(cx));
            }
        })
        .content(move |_, _, cx| picker_body(&spec, cx.entity(), cx))
}

/// The dropdown's single column: the default row, branch rows, tag rows.
fn picker_body(spec: &BasePickerSpec, popover: Entity<PopoverState>, cx: &App) -> AnyElement {
    let branches = spec.refs.iter().filter(|r| !r.tag);
    let tags = spec.refs.iter().filter(|r| r.tag);
    v_flex()
        .id("diff-base-body")
        .test_support()
        .w(px(220.))
        .gap_0p5()
        .child(base_opt(&spec.default, true, spec, &popover, cx))
        .children(branches.map(|r| base_opt(&r.name, false, spec, &popover, cx)))
        .children(tags.map(|r| base_opt(&r.name, false, spec, &popover, cx)))
        .into_any_element()
}

/// One ref row: name, a tag icon for tags, a check on the effective base —
/// the pinned ref, or the default while nothing is pinned (or the pin went
/// stale). Clicking pins the ref on the chat and dismisses the popover.
fn base_opt(name: &str, default: bool, spec: &BasePickerSpec, popover: &Entity<PopoverState>, cx: &App) -> impl IntoElement {
    let current = if default {
        spec.picked.is_none() || spec.stale
    } else {
        spec.picked.as_deref() == Some(name) && !spec.stale
    };
    let pick = (!default).then(|| name.to_string());
    let (ws, popover) = (spec.ws.clone(), popover.clone());
    h_flex()
        .id(SharedString::from(format!("base-opt-{}", if default { "default" } else { name })))
        .test_support()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .hover(|d| d.bg(cx.theme().accent))
        .child(name.to_string())
        .when(default, |d| d.child(div().text_xs().text_color(cx.theme().muted_foreground).child("(default)")))
        .child(div().flex_1())
        .when_some(spec.refs.iter().find(|r| r.tag && r.name == name), |d, _| d.child(IconName::Tag))
        .when(current, |d| d.child(IconName::Check))
        .on_click(move |_, window, cx| {
            ws.update(cx, |this, cx| this.set_diff_base(pick.clone(), cx));
            popover.update(cx, |state, cx| state.dismiss(window, cx));
        })
}
