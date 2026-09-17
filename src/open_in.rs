//! Open-in-editor / reveal-in-Finder for project files: the shared file
//! context menu (Changes rows, explorer rows, the repo-root branch row), the
//! `open`/`open -R`/`open -a` command builders plus the bundled-CLI builders
//! that carry a `file:line` target (diff-row ⌘-click), and the
//! preferred-editor setting they read. Commands run on the background
//! executor so the UI never blocks; a spawn or non-zero exit surfaces as an
//! in-app toast.
//!
//! The process runner is swapped for a recorder under `cfg(test)` — tests
//! assert the exact argv instead of launching real apps.

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::workspace::Workspace;

/// The app "Open in Editor" hands a file to — `Settings.preferred_editor`.
/// `Ask` (the default) expands the menu item into a per-click picker; the
/// concrete editors spawn `open -a <app>`; `Finder` reveals the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreferredEditor {
    VsCode,
    Cursor,
    Zed,
    Finder,
    #[default]
    Ask,
}

impl PreferredEditor {
    /// Every value the General-settings select offers, in display order.
    pub const ALL: [Self; 5] = [Self::VsCode, Self::Cursor, Self::Zed, Self::Finder, Self::Ask];
    /// The concrete editors the `Ask` submenu lists — `Finder` already has
    /// its own menu item and `Ask` can't open anything.
    pub const CHOICES: [Self; 3] = [Self::VsCode, Self::Cursor, Self::Zed];

    /// Persisted name — `Settings.preferred_editor`.
    pub fn name(self) -> &'static str {
        match self {
            Self::VsCode => "vscode",
            Self::Cursor => "cursor",
            Self::Zed => "zed",
            Self::Finder => "finder",
            Self::Ask => "ask",
        }
    }

    /// Select/menu display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::Cursor => "Cursor",
            Self::Zed => "Zed",
            Self::Finder => "Finder",
            Self::Ask => "Ask every time",
        }
    }

    /// Parse a persisted name; unknown or empty falls back to `Ask`.
    pub fn from_name(name: &str) -> Self {
        Self::ALL.into_iter().find(|e| e.name() == name).unwrap_or_default()
    }

    /// Inverse of `label` for the select's Confirm event.
    pub fn from_label(label: &str) -> Self {
        Self::ALL.into_iter().find(|e| e.label() == label).unwrap_or_default()
    }
}

/// One external command to run — the test seam: `run` records these instead
/// of spawning, so tests assert argv without launching real apps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Verb for the failure toast ("Reveal in Finder", "Open in VS Code").
    pub action: String,
}

/// `open -R <abs>` — select the file in a Finder window.
pub fn reveal_command(abs: &std::path::Path) -> OpenCommand {
    OpenCommand {
        program: "open".into(),
        args: vec!["-R".into(), abs.display().to_string()],
        action: "Reveal in Finder".into(),
    }
}

/// `open -a <App> <abs>` for a concrete editor; `Finder` reveals (it isn't
/// an editor) and `Ask` returns `None` — the menu shows a picker instead.
pub fn open_command(editor: PreferredEditor, abs: &std::path::Path) -> Option<OpenCommand> {
    if editor == PreferredEditor::Finder {
        return Some(reveal_command(abs));
    }
    let app = match editor {
        PreferredEditor::VsCode => "Visual Studio Code",
        PreferredEditor::Cursor => "Cursor",
        PreferredEditor::Zed => "Zed",
        PreferredEditor::Finder | PreferredEditor::Ask => return None,
    };
    Some(OpenCommand {
        program: "open".into(),
        args: vec!["-a".into(), app.into(), abs.display().to_string()],
        action: format!("Open in {}", editor.label()),
    })
}

/// `<cli> <abs>:<line>` — the editor's bundled command-line tool, which
/// forwards to a running instance over IPC. VS Code/Cursor take `-g` (goto);
/// Zed's `cli` parses `file:line` positionally. `open -a <App> --args` can't
/// do this job: LaunchServices only passes argv to a *launched* process, so
/// an already-running editor never sees the line.
pub(crate) fn cli_command(editor: PreferredEditor, cli: &std::path::Path, abs: &std::path::Path, line: u32) -> Option<OpenCommand> {
    let target = format!("{}:{line}", abs.display());
    let args = match editor {
        PreferredEditor::VsCode | PreferredEditor::Cursor => vec!["-g".into(), target],
        PreferredEditor::Zed => vec![target],
        PreferredEditor::Finder | PreferredEditor::Ask => return None,
    };
    Some(OpenCommand {
        program: cli.display().to_string(),
        args,
        action: format!("Open in {}", editor.label()),
    })
}

/// The editor's bundled CLI under the standard app locations — `/Applications`
/// and `~/Applications`, the two places `open -a` finds apps by name.
fn bundled_cli(editor: PreferredEditor) -> Option<std::path::PathBuf> {
    let (app, rel) = match editor {
        PreferredEditor::VsCode => ("Visual Studio Code.app", "Contents/Resources/app/bin/code"),
        PreferredEditor::Cursor => ("Cursor.app", "Contents/Resources/app/bin/cursor"),
        PreferredEditor::Zed => ("Zed.app", "Contents/MacOS/cli"),
        PreferredEditor::Finder | PreferredEditor::Ask => return None,
    };
    let mut roots = vec![std::path::PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(std::path::PathBuf::from(home).join("Applications"));
    }
    roots.into_iter().map(|root| root.join(app).join(rel)).find(|cli| cli.is_file())
}

/// `open_command` plus a target line: the bundled CLI carries `file:line`
/// (with `-g` for VS Code/Cursor) to a running instance. Without a CLI —
/// the app isn't under a standard location — VS Code/Cursor fall back to
/// `open -a <App> --args -g <abs>:<line>` (the line survives a cold launch),
/// while Zed's main binary ignores argv and gets the plain open. `Finder`
/// reveals; `Ask` returns `None` like `open_command`.
pub fn open_command_at(editor: PreferredEditor, abs: &std::path::Path, line: u32) -> Option<OpenCommand> {
    if let Some(cli) = bundled_cli(editor) {
        return cli_command(editor, &cli, abs, line);
    }
    let mut cmd = open_command(editor, abs)?;
    if matches!(editor, PreferredEditor::VsCode | PreferredEditor::Cursor) {
        cmd.args = vec!["-a".into(), cmd.args[1].clone(), "--args".into(), "-g".into(), format!("{}:{line}", abs.display())];
    }
    Some(cmd)
}

/// Commands issued during tests, in order — the command fake.
#[cfg(test)]
pub(crate) static ISSUED: parking_lot::Mutex<Vec<OpenCommand>> = parking_lot::Mutex::new(Vec::new());
/// When set, the next `run` fails with this message — the failure-path fake.
#[cfg(test)]
pub(crate) static FAIL_WITH: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

/// Run the command and report spawn/non-zero-exit as `Err`. Under `cfg(test)`
/// nothing spawns: the command is recorded and `FAIL_WITH` decides the result.
#[cfg(not(test))]
fn run(cmd: &OpenCommand) -> Result<(), String> {
    match std::process::Command::new(&cmd.program).args(&cmd.args).output() {
        Err(e) => Err(format!("{e}")),
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if stderr.is_empty() { format!("{} exited {}", cmd.program, out.status) } else { stderr })
        },
    }
}

#[cfg(test)]
fn run(cmd: &OpenCommand) -> Result<(), String> {
    ISSUED.lock().push(cmd.clone());
    match FAIL_WITH.lock().take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Workspace {
    /// `open -R` the project-relative file — selects it in Finder.
    pub fn reveal_in_finder(&mut self, rel: &str, cx: &mut Context<Self>) {
        let abs = self.project.root().join(rel);
        self.reveal_path_in_finder(&abs, cx);
    }

    /// `reveal_in_finder` under an explicit dir — the Changes panel's
    /// worktree rows, whose files live outside the project root.
    pub fn reveal_in_finder_at(&mut self, dir: &std::path::Path, rel: &str, cx: &mut Context<Self>) {
        let abs = dir.join(rel);
        self.reveal_path_in_finder(&abs, cx);
    }

    /// `open -R` an absolute path — the worktree menu's entry point, where
    /// the directory lives outside the project-relative scheme.
    pub fn reveal_path_in_finder(&mut self, abs: &std::path::Path, cx: &mut Context<Self>) {
        let cmd = reveal_command(abs);
        self.run_open_command(cmd, cx);
    }

    /// Open the project-relative file in `editor` (or the preferred editor
    /// when `None`). `Ask` resolves to `None` — the menu shows a picker, so
    /// reaching this with `Ask` is a no-op rather than a guessed app.
    pub fn open_in_editor(&mut self, rel: &str, editor: Option<PreferredEditor>, cx: &mut Context<Self>) {
        let abs = self.project.root().join(rel);
        self.open_path_in_editor(&abs, editor, cx);
    }

    /// `open_in_editor` under an explicit dir — the Changes panel's
    /// worktree rows.
    pub fn open_in_editor_at(&mut self, dir: &std::path::Path, rel: &str, editor: Option<PreferredEditor>, cx: &mut Context<Self>) {
        let abs = dir.join(rel);
        self.open_path_in_editor(&abs, editor, cx);
    }

    /// Open an absolute path in `editor` (or the preferred editor when
    /// `None`) — the worktree menu's entry point. Same `Ask` no-op rule.
    pub fn open_path_in_editor(&mut self, abs: &std::path::Path, editor: Option<PreferredEditor>, cx: &mut Context<Self>) {
        let editor = editor.unwrap_or(self.preferred_editor);
        let Some(cmd) = open_command(editor, abs) else { return };
        self.run_open_command(cmd, cx);
    }

    /// Open an absolute path in `editor` (or the preferred editor when
    /// `None`) at a target line — the diff rows' ⌘-click entry point. `Ask`
    /// can't pick an editor without a menu, so it reveals the file in
    /// Finder instead (same fallback as the conflict rows). Changes-panel
    /// rows pass paths under the changes scope's dir — a worktree for
    /// worktree chats, not always the project root.
    pub fn open_path_in_editor_at(&mut self, abs: &std::path::Path, line: u32, editor: Option<PreferredEditor>, cx: &mut Context<Self>) {
        let editor = editor.unwrap_or(self.preferred_editor);
        let cmd = if editor == PreferredEditor::Ask {
            reveal_command(abs)
        } else {
            let Some(cmd) = open_command_at(editor, abs, line) else { return };
            cmd
        };
        self.run_open_command(cmd, cx);
    }

    /// ⌘-click on a diff row: open the file at the clicked line. Added and
    /// context lines use the new-side number; a removed line falls back to
    /// its old-side number (the file on disk may differ — still the closest
    /// anchor). Rows with no number (hunk headers, "\ No newline") do
    /// nothing.
    pub fn open_diff_at_line(&mut self, file_ix: usize, line_ix: usize, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get(file_ix) else { return };
        let Some(line) = change.diff.as_ref().and_then(|d| d.lines.get(line_ix)) else { return };
        let Some(n) = line.new.or(line.old) else { return };
        let path = self.changes_scope().dir.join(&change.path);
        self.open_path_in_editor_at(&path, n, None, cx);
    }

    /// Route a click on a numbered diff row: ⌘-click opens the file at that
    /// line in the editor; a plain click anchors the review-comment editor
    /// (a no-op on removed lines — `open_review_comment` refuses them).
    /// Shared by the unified rows and both split cells.
    pub fn click_diff_line(&mut self, target: crate::model::ReviewTarget, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.modifiers().secondary() {
            self.open_diff_at_line(target.file_ix, target.line_ix, cx);
        } else {
            self.open_review_comment(target.file_ix, target.line_ix, window, cx);
        }
    }

    /// Copy the file's absolute path under `dir` to the clipboard — `dir`
    /// is the changes scope's dir for worktree rows, the project root
    /// elsewhere.
    pub fn copy_file_path_at(&mut self, dir: &std::path::Path, rel: &str, cx: &mut Context<Self>) {
        let abs = dir.join(rel);
        cx.write_to_clipboard(ClipboardItem::new_string(abs.display().to_string()));
    }

    /// Set the preferred editor and persist it — `Settings.preferred_editor`.
    pub fn set_preferred_editor(&mut self, editor: PreferredEditor, cx: &mut Context<Self>) {
        self.preferred_editor = editor;
        self.save_settings();
        cx.notify();
    }

    /// Spawn the command on the background executor — `open` waits on
    /// LaunchServices and editor CLIs can block, so neither runs on the UI
    /// thread. Failures surface as an in-app error toast.
    fn run_open_command(&mut self, cmd: OpenCommand, cx: &mut Context<Self>) {
        let job = cmd.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { run(&job) }).await;
            if let Err(e) = result {
                notify_failure(this, &cmd.action, &e, cx);
            }
        })
        .detach();
    }
}

/// Post the error toast for a failed open — a free fn so `run_open_command`
/// stays under the nesting lint.
fn notify_failure(this: WeakEntity<Workspace>, action: &str, e: &str, cx: &mut AsyncApp) {
    let message = format!("{action} failed: {e}");
    let _ = this.update_in(cx, |_this, window, cx| {
        window.push_notification(Notification::error(message), cx);
    });
}

/// The shared file context menu — `file_menu` plus the Changes-only "Copy
/// Diff" item — split into `open_in_menu.rs` for the SLOC cap; re-exported
/// so callers keep using `crate::open_in::file_menu` / `copy_diff_item`.
#[path = "open_in_menu.rs"]
mod menu;
pub use menu::{FileTarget, copy_diff_item, explorer_dir_menu, explorer_file_menu, explorer_root_menu, file_menu, file_menu_at};

#[cfg(test)]
#[path = "diff_open_tests.rs"]
mod diff_open_tests;
