//! The conflicts section of the Changes panel — mounted between the review
//! banner and the file list while `Workspace::git.conflicts` is non-empty.
//! A banner counts the unmerged paths; one row per file offers "Use ours" /
//! "Use theirs" (checkout + add), "Open in editor" (edit the markers by
//! hand), and a "Mark resolved" check (`git add`). Ops live in
//! `crate::changes_conflicts`; this file only renders.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::changes_conflicts::ConflictSide;
use crate::workspace::Workspace;

/// What a conflict row's chips do — kept as data so `chip` can share the
/// button chrome across all four actions.
#[derive(Clone, Copy)]
enum ConflictAction {
    Ours,
    Theirs,
    Edit,
    Resolved,
}

impl ConflictAction {
    /// Dispatch to the `Workspace` op — `ix` is the row's index in
    /// `git.conflicts`.
    fn run(self, ws: &mut Workspace, ix: usize, cx: &mut Context<Workspace>) {
        match self {
            Self::Ours => ws.resolve_conflict(ix, ConflictSide::Ours, cx),
            Self::Theirs => ws.resolve_conflict(ix, ConflictSide::Theirs, cx),
            Self::Edit => ws.open_conflict_in_editor(ix, cx),
            Self::Resolved => ws.mark_conflict_resolved(ix, cx),
        }
    }
}

/// Owned inputs for one chip — `conflict_row` builds these so the click
/// closure holds no borrow of the workspace.
#[derive(Clone, Copy)]
struct ChipSpec {
    id: &'static str,
    ix: usize,
    label: &'static str,
    icon: IconName,
    /// Git ops are refused while another op runs; "Open in editor" isn't a
    /// git op and stays enabled.
    enabled: bool,
    action: ConflictAction,
}

/// The section: banner line plus one row per conflicted path.
pub fn conflicts_section(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let n = ws.git.conflicts.len();
    let banner = format!("{n} conflict{} — resolve to continue the merge", if n == 1 { "" } else { "s" });
    let rows = ws
        .git
        .conflicts
        .iter()
        .enumerate()
        .map(|(ix, path)| conflict_row(ix, path, ws.git.busy, cx))
        .collect::<Vec<_>>();
    v_flex()
        .id("conflicts-section")
        .test_support()
        .border_b_1()
        .border_color(cx.theme().border)
        .px_3()
        .py_2()
        .gap_1()
        .child(
            h_flex()
                .id("conflicts-banner")
                .test_support()
                .aria_label(banner.clone())
                .gap_2()
                .text_xs()
                .child(div().text_color(cx.theme().danger).child(IconName::CircleAlert))
                .child(div().text_color(cx.theme().danger).child(banner)),
        )
        .children(rows)
        .into_any_element()
}

/// One conflicted file: the path line, then the action chips. Right-click
/// opens the same file menu as a Changes row.
fn conflict_row(ix: usize, path: &str, busy: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let ws = cx.entity();
    let chips = [
        ChipSpec {
            id: "conflict-ours",
            ix,
            label: "Use ours",
            icon: IconName::Check,
            enabled: !busy,
            action: ConflictAction::Ours,
        },
        ChipSpec {
            id: "conflict-theirs",
            ix,
            label: "Use theirs",
            icon: IconName::Check,
            enabled: !busy,
            action: ConflictAction::Theirs,
        },
        ChipSpec {
            id: "conflict-edit",
            ix,
            label: "Open in editor",
            icon: IconName::ExternalLink,
            enabled: true,
            action: ConflictAction::Edit,
        },
        ChipSpec {
            id: "conflict-resolved",
            ix,
            label: "Mark resolved",
            icon: IconName::SquareCheck,
            enabled: !busy,
            action: ConflictAction::Resolved,
        },
    ];
    v_flex()
        .id(("conflict-row", ix))
        .test_support()
        .gap_0p5()
        .py_0p5()
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().flex_shrink_0().text_color(cx.theme().danger).child(IconName::CircleAlert))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis_middle()
                        .child(path.to_string()),
                ),
        )
        .child(h_flex().gap_1().pl_6().flex_wrap().children(chips.into_iter().map(|spec| chip(spec, &ws, cx))))
        .context_menu({
            let ws = cx.entity();
            let path = path.to_string();
            move |menu, window, cx| crate::open_in::file_menu(&ws, &path, menu, window, cx)
        })
        .into_any_element()
}

/// One bordered action chip — muted and inert while `enabled` is false.
fn chip(spec: ChipSpec, ws: &Entity<Workspace>, cx: &App) -> AnyElement {
    let (border, muted_fg, fg) = {
        let theme = cx.theme();
        (theme.border, theme.muted_foreground, theme.foreground)
    };
    let ws = ws.clone();
    div()
        .id((spec.id, spec.ix))
        .test_support()
        .aria_label(spec.label)
        .flex()
        .items_center()
        .gap_1()
        .rounded_md()
        .px_2()
        .py_0p5()
        .text_xs()
        .border_1()
        .border_color(border)
        .text_color(if spec.enabled { fg } else { muted_fg })
        .when(spec.enabled, |d| {
            d.cursor_pointer().hover(|d| d.bg(muted_fg.opacity(0.15))).on_click(move |_, _, cx| {
                ws.update(cx, |this, cx| spec.action.run(this, spec.ix, cx));
            })
        })
        .child(spec.icon)
        .child(spec.label)
        .into_any_element()
}
