//! File explorer — the sidebar's Files tab. A tree built from the same
//! `project_files` scan that feeds the @-mention picker: directories
//! expand/collapse, clicking a file inserts `@path ` into the composer
//! (there is no editor surface to open files in — the mention is how the
//! explorer hands a file to the agent), and the context menus create,
//! rename and delete entries on disk (see `crate::files::fs_ops`).

mod rows;

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::files::build_file_tree;
use crate::views::explorer_git;
use crate::views::sidebar::SidebarTab;
use crate::workspace::Workspace;

/// Explorer panel state: expanded directory paths (project-relative), the
/// last file clicked (its row stays highlighted), whether top-level dirs
/// were auto-expanded on first render, and the in-flight inline edit.
#[derive(Default)]
pub struct ExplorerState {
    pub expanded: std::collections::HashSet<String>,
    pub selected: Option<String>,
    pub seeded: bool,
    /// `Some` while the tree's inline name input is armed — see
    /// `crate::files::fs_ops` for the ops it commits.
    pub editing: Option<ExplorerEdit>,
}

/// The inline edit the tree's input row is armed for. `dir` is the parent
/// the new entry lands in ("" = project root); `path` is the renamed row.
pub(crate) enum ExplorerEdit {
    NewFile { dir: String },
    NewFolder { dir: String },
    Rename { path: String },
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
        // Git badges are a view over the workspace's `changes` snapshot —
        // built once per render, never a fresh `git status`.
        let git = explorer_git::GitDecorations::build(&self.changes);
        let mut rows = Vec::new();
        let flat = rows::Flat {
            expanded: &self.explorer.expanded,
            git: &git,
            editing: self.explorer.editing.as_ref(),
        };
        rows::flatten(&tree, 0, &flat, &mut rows);
        let selected = self.explorer.selected.clone();
        let input = self.explorer_input.clone();
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| rows::render_row(ix, row, selected.as_deref(), &input, cx))
            .collect();
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
                    .id("explorer-header")
                    .test_support()
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
                    )
                    .context_menu({
                        let ws = cx.entity();
                        move |menu, window, cx| crate::open_in::explorer_root_menu(&ws, menu, window, cx)
                    }),
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
