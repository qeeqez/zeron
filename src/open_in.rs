//! Open-in-editor / reveal-in-Finder for project files: the shared file
//! context menu (Changes rows, explorer rows, the repo-root branch row), the
//! `open`/`open -R`/`open -a` command builders, and the preferred-editor
//! setting they read. Commands run on the background executor so the UI
//! never blocks; a spawn or non-zero exit surfaces as an in-app toast.
//!
//! The process runner is swapped for a recorder under `cfg(test)` — tests
//! assert the exact argv instead of launching real apps.

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
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
    pub program: &'static str,
    pub args: Vec<String>,
    /// Verb for the failure toast ("Reveal in Finder", "Open in VS Code").
    pub action: String,
}

/// `open -R <abs>` — select the file in a Finder window.
pub fn reveal_command(abs: &std::path::Path) -> OpenCommand {
    OpenCommand {
        program: "open",
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
        program: "open",
        args: vec!["-a".into(), app.into(), abs.display().to_string()],
        action: format!("Open in {}", editor.label()),
    })
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
    match std::process::Command::new(cmd.program).args(&cmd.args).output() {
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
        let cmd = reveal_command(&self.project.root().join(rel));
        self.run_open_command(cmd, cx);
    }

    /// Open the project-relative file in `editor` (or the preferred editor
    /// when `None`). `Ask` resolves to `None` — the menu shows a picker, so
    /// reaching this with `Ask` is a no-op rather than a guessed app.
    pub fn open_in_editor(&mut self, rel: &str, editor: Option<PreferredEditor>, cx: &mut Context<Self>) {
        let editor = editor.unwrap_or(self.preferred_editor);
        let Some(cmd) = open_command(editor, &self.project.root().join(rel)) else { return };
        self.run_open_command(cmd, cx);
    }

    /// Copy the file's absolute path to the clipboard.
    pub fn copy_file_path(&mut self, rel: &str, cx: &mut Context<Self>) {
        let abs = self.project.root().join(rel);
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

/// One "Open in <editor>" item for the `Ask` submenu — a free fn so the
/// submenu fold stays under the nesting lint.
fn editor_pick_item(ws: &Entity<Workspace>, rel: &str, editor: PreferredEditor) -> PopupMenuItem {
    let ws = ws.clone();
    let rel = rel.to_string();
    PopupMenuItem::new(editor.label()).on_click(move |_, _w, cx| {
        ws.update(cx, |this, cx| this.open_in_editor(&rel, Some(editor), cx));
    })
}

/// The right-click menu shared by every file row: Changes entries, explorer
/// files, and — with `rel` empty — the repo root on the git branch row.
/// "Open in Editor" reads the preferred editor; `Ask` turns the item into a
/// submenu of concrete editors.
pub fn file_menu(ws: &Entity<Workspace>, rel: &str, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let preferred = ws.read(cx).preferred_editor;
    let ws_reveal = ws.clone();
    let rel_reveal = rel.to_string();
    let menu = menu.item(PopupMenuItem::new("Reveal in Finder").icon(IconName::FolderOpen).on_click(move |_, _w, cx| {
        ws_reveal.update(cx, |this, cx| this.reveal_in_finder(&rel_reveal, cx));
    }));
    let menu =
        if preferred == PreferredEditor::Ask {
            let ws_pick = ws.clone();
            let rel_pick = rel.to_string();
            menu.submenu("Open in Editor", window, cx, move |m, _w, _cx| {
                PreferredEditor::CHOICES
                    .into_iter()
                    .fold(m, |m, editor| m.item(editor_pick_item(&ws_pick, &rel_pick, editor)))
            })
        } else {
            let ws_open = ws.clone();
            let rel_open = rel.to_string();
            menu.item(PopupMenuItem::new(format!("Open in {}", preferred.label())).icon(IconName::ExternalLink).on_click(
                move |_, _w, cx| {
                    ws_open.update(cx, |this, cx| this.open_in_editor(&rel_open, None, cx));
                },
            ))
        };
    let ws_copy = ws.clone();
    let rel_copy = rel.to_string();
    menu.item(PopupMenuItem::new("Copy Path").icon(IconName::Copy).on_click(move |_, _w, cx| {
        ws_copy.update(cx, |this, cx| this.copy_file_path(&rel_copy, cx));
    }))
}
