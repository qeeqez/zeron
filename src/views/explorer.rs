//! File explorer — the sidebar's Files tab. A read-only tree built from the
//! same `project_files` scan that feeds the @-mention picker: directories
//! expand/collapse, and clicking a file inserts `@path ` into the composer
//! (there is no editor surface to open files in — the mention is how the
//! explorer hands a file to the agent).

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::files::{DirNode, build_file_tree};
use crate::views::sidebar::SidebarTab;
use crate::workspace::Workspace;

/// Explorer panel state: expanded directory paths (project-relative), the
/// last file clicked (its row stays highlighted), and whether top-level
/// dirs were auto-expanded on first render.
#[derive(Default)]
pub struct ExplorerState {
    pub expanded: std::collections::HashSet<String>,
    pub selected: Option<String>,
    pub seeded: bool,
}

/// One visible line of the flattened tree.
enum Row {
    Dir { name: SharedString, path: SharedString, depth: usize, expanded: bool },
    File { path: SharedString, depth: usize },
}

impl Workspace {
    /// Cmd-Shift-E / View > Toggle Explorer: reveal the Files tab — opening
    /// the sidebar first when it's collapsed — or fall back to Chats when
    /// Files is already showing.
    pub fn toggle_explorer(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_collapsed {
            self.sidebar_collapsed = false;
            self.sidebar_tab = SidebarTab::Files;
            self.save_settings();
        } else {
            self.sidebar_tab = match self.sidebar_tab {
                SidebarTab::Files => SidebarTab::Chats,
                SidebarTab::Chats => SidebarTab::Files,
            };
        }
        cx.notify();
    }

    /// Expand or collapse a directory row.
    pub fn toggle_explorer_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.explorer.expanded.remove(path) {
            self.explorer.expanded.insert(path.to_string());
        }
        cx.notify();
    }

    /// Click a file: drop `@path ` into the composer and focus it. An open
    /// mention query (`…@par|`) is replaced; otherwise the mention appends
    /// to the draft — same replacement rule as the @-picker's apply.
    pub fn mention_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer.selected = Some(path.to_string());
        self.composer.update(cx, |s, cx| {
            s.set_value(mention_text(&s.value(), path), window, cx);
            s.focus(window, cx);
        });
        // `set_value` suppresses Change — nudge so the mention menu closes.
        cx.notify();
    }

    /// Re-scan the project tree — the refresh button. Same background
    /// executor hop as the launch scan in `lifecycle::start_background`.
    pub fn refresh_project_files(&mut self, cx: &mut Context<Self>) {
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { crate::files::scan_project_files(&root) }).await;
            let _ = this.update(cx, |this, cx| {
                this.project_files = files;
                cx.notify();
            });
        })
        .detach();
    }

    /// The sidebar's Files tab: a scrollable tree column sized like the chat
    /// list. `header`/`footer` are the shared sidebar chrome (title + tab
    /// strip up top, account/settings row at the bottom).
    pub fn render_explorer(&mut self, header: AnyElement, footer: AnyElement, cx: &mut Context<Self>) -> impl IntoElement {
        // First render after the scan lands expands the top-level dirs so
        // the tree isn't a wall of collapsed rows; user toggles after that
        // are kept (`explorer.seeded` gates the seeding).
        if !self.explorer.seeded && !self.project_files.is_empty() {
            self.explorer.seeded = true;
            self.explorer
                .expanded
                .extend(build_file_tree(&self.project_files).dirs.iter().map(|d| d.path.to_string()));
        }
        let tree = build_file_tree(&self.project_files);
        let mut rows = Vec::new();
        flatten(&tree, 0, &self.explorer.expanded, &mut rows);
        let selected = self.explorer.selected.clone();
        let rows: Vec<AnyElement> = rows.into_iter().enumerate().map(|(ix, row)| render_row(ix, row, selected.as_deref(), cx)).collect();
        div()
            .id("explorer")
            .test_support()
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .text_color(cx.theme().sidebar_foreground)
            .child(h_flex().id("header").pt_3().px_3().gap_2().child(header))
            .child(
                h_flex()
                    .px_3()
                    .pt_1()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} files", self.project_files.len()))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("explorer-refresh")
                            .test_support()
                            .cursor_pointer()
                            .child(IconName::RefreshCcw)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_project_files(cx))),
                    ),
            )
            .child(
                div()
                    .id("explorer-tree")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .when(rows.is_empty(), |d| {
                        d.child(div().text_sm().text_color(cx.theme().muted_foreground).child("No files — open a project folder"))
                    })
                    .children(rows),
            )
            .child(h_flex().id("footer").pb_3().px_3().gap_2().child(footer))
    }
}

/// Depth-first walk: a dir emits its row, then its children when expanded.
fn flatten(dir: &DirNode, depth: usize, expanded: &std::collections::HashSet<String>, out: &mut Vec<Row>) {
    for d in &dir.dirs {
        let open = expanded.contains(d.path.as_str());
        out.push(Row::Dir {
            name: d.name.clone(),
            path: d.path.clone(),
            depth,
            expanded: open,
        });
        if open {
            flatten(d, depth + 1, expanded, out);
        }
    }
    for f in &dir.files {
        out.push(Row::File { path: f.clone(), depth });
    }
}

fn render_row(ix: usize, row: Row, selected: Option<&str>, cx: &mut Context<Workspace>) -> AnyElement {
    match row {
        Row::Dir { name, path, depth, expanded } => {
            let indent = 8. + depth as f32 * 14.;
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
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_explorer_dir(&path, cx)))
                .into_any_element()
        },
        Row::File { path, depth } => {
            let indent = 8. + depth as f32 * 14. + 16.;
            let is_selected = selected == Some(path.as_str());
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
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(file_name(&path).to_string()),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.mention_file(&path, window, cx)))
                .into_any_element()
        },
    }
}

/// The composer text after picking `path`: an open mention query — a `@`
/// at a word boundary with a whitespace-free tail — is replaced, otherwise
/// the mention appends (padding the draft with a space when needed).
pub(crate) fn mention_text(current: &str, path: &str) -> String {
    if let Some((before, query)) = current.rsplit_once('@')
        && before.chars().next_back().is_none_or(char::is_whitespace)
        && !query.chars().any(char::is_whitespace)
    {
        return format!("{before}@{path} ");
    }
    if current.is_empty() || current.ends_with(char::is_whitespace) {
        format!("{current}@{path} ")
    } else {
        format!("{current} @{path} ")
    }
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
