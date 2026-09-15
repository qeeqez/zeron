//! Go-to-file palette (Cmd-P): a fuzzy picker over `project_files` — the
//! same list the @-mention menu scans — that drops the pick into the
//! composer as an @-mention, matching Codex's cmd-p.
//!
//! Like the command palette, the `Command` is `filterable(false)` and
//! ranking lives here: fuzzy score over the project-relative path, with
//! recent picks breaking ties and leading the empty-query list. The dialog
//! builder runs while `Workspace::render` holds the entity lease, so it
//! works from snapshots taken at open time; each keystroke re-runs the
//! builder via `on_query` → `cx.notify()`.

use gpui_kit::assets::IconName;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{IndexPath, WindowExt};
use gpui_kit::*;

use crate::workspace::Workspace;

/// Most rows the picker lists — the scan already caps candidates, this
/// keeps the ranked vec small on huge trees.
const MAX_ROWS: usize = 200;
/// Recent picks remembered for ranking — enough to cover a work session.
const MAX_RECENT: usize = 20;

/// `files` ranked for `query`: an empty query lists recent picks first,
/// then everything in scan order; a query keeps fuzzy matches, best score
/// first, recency breaking ties.
pub(crate) fn rank_files(files: &[SharedString], recent: &[SharedString], query: &str) -> Vec<SharedString> {
    let query = query.trim();
    let mut ranked: Vec<(usize, i32)> = files
        .iter()
        .enumerate()
        .filter_map(|(ix, f)| {
            if query.is_empty() {
                Some((ix, 0))
            } else {
                crate::palette_fuzzy::fuzzy_score(query, f).map(|s| (ix, s))
            }
        })
        .collect();
    let recency = |ix: usize| recent.iter().position(|r| *r == files[ix]).unwrap_or(usize::MAX);
    if query.is_empty() {
        // Stable sort: non-recent files keep scan order behind the picks.
        ranked.sort_by_key(|(ix, _)| recency(*ix));
    } else {
        ranked.sort_by_key(|(ix, score)| (std::cmp::Reverse(*score), recency(*ix)));
    }
    ranked.into_iter().take(MAX_ROWS).map(|(ix, _)| files[ix].clone()).collect()
}

/// A file row: extension icon + the project-relative path.
fn file_item(path: SharedString) -> CommandItem {
    CommandItem::new().label(path.clone()).icon(file_icon(&path))
}

/// Icon by extension — mirrors the explorer's map so a file reads the same
/// in both pickers.
fn file_icon(path: &str) -> IconName {
    match path.rsplit('.').next().unwrap_or_default() {
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

/// The picker's `Command` element, rebuilt on every workspace render —
/// `on_query` notifies so each keystroke re-ranks against the snapshot.
fn file_command(
    state: &Entity<CommandState>, files: &[SharedString], recent: &[SharedString], ws: &Entity<Workspace>, cx: &mut App,
) -> Command {
    let ws_confirm = ws.clone();
    let ws_query = ws.clone();
    let ranked = rank_files(files, recent, &state.read(cx).query(cx));
    let group = CommandGroup::new().label("Files").items(ranked.into_iter().map(file_item));
    Command::new(state)
        .placeholder("Go to file…")
        // Ranking happens in `rank_files`, not the component's substring filter.
        .filterable(false)
        .group(group)
        .empty(|state, _, cx| {
            let hint = if state.query(cx).trim().is_empty() { "No files in this project" } else { "No matching files" };
            div().py_6().w_full().text_center().text_sm().text_color(cx.theme().muted_foreground).child(hint)
        })
        .footer(|_, _, cx| crate::palette::command_footer("↵ mention", cx))
        .on_query(move |_, _, cx| {
            ws_query.update(cx, |_, cx| cx.notify());
        })
        .on_confirm(move |path, window, cx| {
            ws_confirm.update(cx, |this, cx| this.confirm_file_pick(path, window, cx));
        })
        .on_cancel(|window, cx| window.close_dialog(cx))
}

impl Workspace {
    /// Cmd-P: fuzzy file picker. Pressing it again (or with any dialog up)
    /// closes the dialog, like the command palette.
    pub fn open_file_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
            return;
        }
        // Fresh query each open — the state entity persists across dialogs.
        self.file_palette.update(cx, |state, cx| state.set_query("", window, cx));
        // Snapshot for the dialog builder: it runs during `render` while the
        // workspace lease is held, so it can't read `self`.
        let files = self.project_files.clone();
        let recent = self.recent_files.clone();
        let state = self.file_palette.clone();
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .close_button(false)
                .overlay_closable(true)
                .child(file_command(&state, &files, &recent, &ws, cx))
        });
        // The dialog focuses its own handle on open; the query field needs
        // focus so typing and ↑↓/Enter reach the Command context.
        self.file_palette.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Resolve a confirmed row to its file and drop it into the composer as
    /// an @-mention — the same path the explorer's file click takes. Runs
    /// after the dialog closes so focus lands back on the composer.
    fn confirm_file_pick(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let query = self.file_palette.read(cx).query(cx);
        let Some(file) = rank_files(&self.project_files, &self.recent_files, &query).get(path.row).cloned() else {
            return;
        };
        self.recent_files.retain(|f| f != &file);
        self.recent_files.insert(0, file.clone());
        self.recent_files.truncate(MAX_RECENT);
        self.mention_file(&file, window, cx);
    }
}
