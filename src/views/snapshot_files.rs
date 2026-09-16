//! The expanded snapshot row's file list — every path restore would
//! rewrite or remove, with the same status icons the Changes panel uses.
//! Split from `snapshots.rs` for the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::{ActiveTheme, Theme};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::snapshot_store::{SnapshotFile, SnapshotStatus};
use crate::snapshots::{SnapshotFiles, SnapshotInfo};
use crate::workspace::Workspace;

/// The list under an expanded row: the loaded paths, or the
/// loading/failed/no-op note for the other states.
pub(super) fn snapshot_files(ix: usize, snap: &SnapshotInfo, cx: &mut Context<Workspace>) -> impl IntoElement {
    let theme = cx.theme();
    let mut list = div()
        .id(("snapshot-files", ix))
        .test_support()
        .flex()
        .flex_col()
        .pl_8()
        .pr_2()
        .pb_1()
        .text_xs()
        .text_color(theme.muted_foreground);
    match &snap.files {
        SnapshotFiles::Loaded(files) if files.is_empty() => {
            list = list.child("No changes — restore is a no-op");
        },
        SnapshotFiles::Loaded(files) => {
            for file in files {
                list = list.child(file_line(file, theme));
            }
        },
        SnapshotFiles::Failed => {
            list = list.child("Couldn't compute the changed files");
        },
        _ => {
            list = list.child("Loading…");
        },
    }
    list
}

/// One path in the expanded list: status icon + the workdir-relative path.
fn file_line(file: &SnapshotFile, theme: &Theme) -> impl IntoElement {
    let (icon, color) = match file.status {
        SnapshotStatus::Added => (IconName::FilePlus, theme.success),
        SnapshotStatus::Modified => (IconName::FilePen, theme.warning),
        SnapshotStatus::Deleted => (IconName::FileX, theme.danger),
    };
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .py_0p5()
        .child(div().flex_shrink_0().text_color(color).child(icon))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis_middle()
                .child(file.path.clone()),
        )
}
