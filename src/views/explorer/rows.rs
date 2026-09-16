//! Explorer row rendering — the flattened tree rows plus the inline edit
//! row (new file/folder, rename). Split from `explorer.rs` for the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Input, InputState};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::files::DirNode;
use crate::views::explorer::ExplorerEdit;
use crate::views::explorer_git;
use crate::workspace::Workspace;

/// One visible line of the flattened tree.
pub(super) enum Row {
    Dir {
        name: SharedString,
        path: SharedString,
        depth: usize,
        expanded: bool,
        dirty: Option<explorer_git::Tone>,
    },
    File {
        path: SharedString,
        depth: usize,
        badge: Option<explorer_git::GitBadge>,
    },
    /// The inline name input — a create row sits at the top of its parent's
    /// children; a rename row replaces the renamed row.
    Edit { depth: usize, icon: IconName },
}

/// Shared context for the depth-first walk — the expanded set, git
/// decorations, and the armed edit — bundled so `flatten` stays under the
/// argument lint.
pub(super) struct Flat<'a> {
    pub expanded: &'a std::collections::HashSet<String>,
    pub git: &'a explorer_git::GitDecorations,
    pub editing: Option<&'a ExplorerEdit>,
}

/// Depth-first walk: a dir emits its row, then its children when expanded.
/// A create edit inserts its input row at the top of the parent's children;
/// a rename swaps the target's row for the input (a renamed dir's children
/// hide — their paths are stale mid-rename).
pub(super) fn flatten(dir: &DirNode, depth: usize, flat: &Flat<'_>, out: &mut Vec<Row>) {
    if let Some(ExplorerEdit::NewFile { dir: d } | ExplorerEdit::NewFolder { dir: d }) = flat.editing
        && d.as_str() == dir.path.as_str()
    {
        let icon = if matches!(flat.editing, Some(ExplorerEdit::NewFolder { .. })) {
            IconName::Folder
        } else {
            IconName::File
        };
        out.push(Row::Edit { depth, icon });
    }
    for d in &dir.dirs {
        if matches!(flat.editing, Some(ExplorerEdit::Rename { path }) if path.as_str() == d.path.as_str()) {
            out.push(Row::Edit { depth, icon: IconName::Folder });
            continue;
        }
        let open = flat.expanded.contains(d.path.as_str());
        out.push(Row::Dir {
            name: d.name.clone(),
            path: d.path.clone(),
            depth,
            expanded: open,
            dirty: flat.git.dir(&d.path),
        });
        if open {
            flatten(d, depth + 1, flat, out);
        }
    }
    for f in &dir.files {
        if matches!(flat.editing, Some(ExplorerEdit::Rename { path }) if path.as_str() == f.as_str()) {
            out.push(Row::Edit { depth, icon: file_icon(f) });
        } else {
            out.push(Row::File { path: f.clone(), depth, badge: flat.git.file(f) });
        }
    }
}

pub(super) fn render_row(
    ix: usize, row: Row, selected: Option<&str>, input: &Entity<InputState>, cx: &mut Context<Workspace>,
) -> AnyElement {
    match row {
        Row::Dir { name, path, depth, expanded, dirty } => {
            let indent = 8. + depth as f32 * 14.;
            let menu_path = path.to_string();
            let ws = cx.entity();
            div()
                .id(("explorer-dir", ix))
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .pl(px(indent))
                .pr_2()
                .py_0p5()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .hover(|d| d.bg(cx.theme().muted))
                .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                }))
                .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(if expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                }))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                .when_some(dirty.map(|tone| explorer_git::dirty_dot(ix, tone, cx)), |d, dot| d.child(dot))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_explorer_dir(&path, cx)))
                .context_menu(move |menu, window, cx| crate::open_in::explorer_dir_menu(&ws, &menu_path, menu, window, cx))
                .into_any_element()
        },
        Row::File { path, depth, badge } => {
            let indent = 8. + depth as f32 * 14. + 16.;
            let is_selected = selected == Some(path.as_str());
            let menu_path = path.to_string();
            let ws = cx.entity();
            div()
                .id(("explorer-file", ix))
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .pl(px(indent))
                .pr_2()
                .py_0p5()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(is_selected, |d| d.bg(cx.theme().accent))
                .when(!is_selected, |d| d.hover(|d| d.bg(cx.theme().muted)))
                .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(file_icon(&path)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(file_name(&path).to_string()),
                )
                .when_some(badge.map(|b| explorer_git::badge_element(ix, b, cx)), |d, el| d.child(el))
                .on_click(cx.listener(move |this, _, window, cx| this.mention_file(&path, window, cx)))
                .context_menu(move |menu, window, cx| crate::open_in::explorer_file_menu(&ws, &menu_path, menu, window, cx))
                .into_any_element()
        },
        Row::Edit { depth, icon } => edit_row(ix, depth, icon, input, cx),
    }
}

/// The inline name input — Enter commits, Escape cancels, a mouse-down
/// anywhere outside commits (Finder-style, same as the chat row's editor).
fn edit_row(ix: usize, depth: usize, icon: IconName, input: &Entity<InputState>, cx: &mut Context<Workspace>) -> AnyElement {
    let indent = 8. + depth as f32 * 14. + 16.;
    let ws_out = cx.entity();
    let ws_enter = cx.entity();
    let ws_esc = cx.entity();
    div()
        .id(("explorer-edit", ix))
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .pl(px(indent))
        .pr_2()
        .py_0p5()
        .rounded_md()
        .text_sm()
        .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child(icon))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .on_mouse_down_out(move |_, window, cx| {
                    ws_out.update(cx, |this, cx| this.commit_explorer_edit(window, cx));
                })
                .on_action(move |_: &InputEnter, window, cx| {
                    cx.stop_propagation();
                    ws_enter.update(cx, |this, cx| this.commit_explorer_edit(window, cx));
                })
                .on_action(move |_: &InputEscape, window, cx| {
                    cx.stop_propagation();
                    ws_esc.update(cx, |this, cx| this.cancel_explorer_edit(window, cx));
                })
                .child(Input::new(input).id("explorer-input").xsmall().w_full()),
        )
        .into_any_element()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Icon by extension — a compact map, not a mime table: code, data, prose,
/// media, archives, and a generic fallback.
fn file_icon(path: &str) -> IconName {
    match file_name(path).rsplit('.').next().unwrap_or_default() {
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "c" | "h" | "cc" | "cpp" | "java" | "rb" | "swift" | "kt" | "sh" | "css"
        | "scss" | "html" | "vue" | "svelte" => IconName::FileCode,
        "json" | "jsonc" | "yaml" | "yml" | "toml" | "xml" => IconName::FileBraces,
        "md" | "txt" | "rtf" => IconName::FileText,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" => IconName::FileImage,
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" => IconName::FileArchive,
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => IconName::FileMusic,
        "mp4" | "mov" | "mkv" | "webm" | "avi" => IconName::FileVideoCamera,
        "lock" | "sqlite" | "db" => IconName::FileBox,
        _ => IconName::File,
    }
}
