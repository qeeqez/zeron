//! The project switcher — a dialog listing recent project folders plus an
//! "Open Folder…" row for the native picker. Opened from the sidebar's
//! project row; picking an entry opens that project in its own window (or
//! focuses the window already bound to it — see `lifecycle::open_project`).

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Icon, WindowExt, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// The sidebar project row's dialog: recent folders + Open Folder….
    /// Recents are snapshotted at open — the list is rebuilt on the next
    /// open, so a folder deleted mid-session can't linger past it.
    pub fn open_project_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let recents = crate::recent_projects::list();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog.title("Switch Project").overlay_closable(true).child(
                v_flex()
                    .id("project-switcher")
                    .test_support()
                    .w(px(340.))
                    .gap_1()
                    .child(open_folder_row(cx))
                    .children(recents.iter().map(|root| recent_row("project-recent", root.clone(), cx))),
            )
        });
    }
}

/// The sidebar header's project row — current folder name + a switcher
/// chevron; clicking opens `open_project_switcher`.
pub(crate) fn project_button(name: &str, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("project-switcher-btn")
        .test_support()
        .flex()
        .items_center()
        .gap_1p5()
        .px_1p5()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .hover(|d| d.bg(cx.theme().muted).text_color(cx.theme().foreground))
        .child(IconName::Folder)
        .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(name.to_string()))
        .child(IconName::ChevronsUpDown)
        .on_click(cx.listener(|this, _, window, cx| this.open_project_switcher(window, cx)))
}

/// The "Open Folder…" row — closes the dialog, then runs the native picker.
fn open_folder_row(cx: &App) -> impl IntoElement {
    div()
        .id("project-open-folder")
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(Icon::new(IconName::FolderOpen).size_4())
        .child("Open Folder…")
        .on_click(|_, window, cx| {
            window.close_dialog(cx);
            crate::lifecycle::prompt_open_project(cx);
        })
}

/// One recent-project row: folder icon, folder name, dimmed full path.
/// Clicking opens the project in its own window (or focuses the existing
/// one). Shared by the switcher dialog and the empty state — `id_prefix`
/// keeps the two surfaces' element ids distinct for tests.
pub(crate) fn recent_row(id_prefix: &str, root: PathBuf, cx: &App) -> impl IntoElement {
    let name = root
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let path = root.display().to_string();
    div()
        .id(SharedString::from(format!("{id_prefix}-{path}")))
        .test_support()
        .flex()
        .items_center()
        .px_2()
        .py_1p5()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(Icon::new(IconName::Folder).size_4().text_color(cx.theme().muted_foreground))
        .child(
            v_flex().flex_1().min_w_0().child(div().overflow_hidden().text_ellipsis().child(name)).child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(path),
            ),
        )
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            crate::lifecycle::open_project(&root, cx);
        })
}
