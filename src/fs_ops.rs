//! Explorer file operations — new file/folder, rename, delete — all routed
//! through `FsOp` so every path is validated project-relative before it
//! reaches `std::fs`. Ops run on the background executor; a success re-runs
//! the project scan (the tree picks up the change) and a failure lands as an
//! error toast.
//!
//! The disk call is swapped for a recorder under `cfg(test)` — tests assert
//! the dispatched op instead of touching the real project (the test
//! workspace's root is the repo itself), same seam as `open_in::run`.

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::views::explorer::ExplorerEdit;
use crate::workspace::Workspace;

/// One filesystem op against the project. Paths are project-relative and
/// pass `relative_path` before `perform` joins them onto the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FsOp {
    /// Create an empty file (refuses to clobber an existing path).
    NewFile(String),
    /// Create a directory (and any missing parents).
    NewFolder(String),
    Rename {
        old: String,
        new: String,
    },
    /// `is_dir` picks `remove_dir_all` over `remove_file`.
    Delete {
        path: String,
        is_dir: bool,
    },
}

impl FsOp {
    /// The toast prefix when the op fails — "Create file failed: …".
    fn action(&self) -> &'static str {
        match self {
            Self::NewFile(_) => "Create file",
            Self::NewFolder(_) => "Create folder",
            Self::Rename { .. } => "Rename",
            Self::Delete { .. } => "Delete",
        }
    }
}

/// `s` as a project-relative path, or `None` when it would escape the
/// project (absolute, `..`, drive letters) or names nothing (empty, `.`,
/// empty segments). `Component::Normal` also drops interior `./` segments,
/// so the joined path is already normalized.
pub(crate) fn relative_path(s: &str) -> Option<&std::path::Path> {
    if s.is_empty() || s.contains(':') || s.split('/').any(|seg| seg.is_empty()) {
        return None;
    }
    let path = std::path::Path::new(s);
    if path.is_absolute() || !path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    Some(path)
}

/// Run `op` against `root`. Every path re-validates through `relative_path`
/// and the join must stay under `root` — a belt-and-suspenders check so no
/// caller mistake can touch files outside the project.
pub(crate) fn perform(root: &std::path::Path, op: &FsOp) -> Result<(), String> {
    let join = |rel: &str| -> Result<std::path::PathBuf, String> {
        let rel = relative_path(rel).ok_or_else(|| format!("“{rel}” isn't a project-relative path"))?;
        let abs = root.join(rel);
        if !abs.starts_with(root) {
            return Err(format!("“{}” escapes the project", rel.display()));
        }
        Ok(abs)
    };
    match op {
        FsOp::NewFile(rel) => {
            let abs = join(rel)?;
            if let Some(dir) = abs.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&abs)
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        FsOp::NewFolder(rel) => std::fs::create_dir_all(join(rel)?).map_err(|e| e.to_string()),
        FsOp::Rename { old, new } => std::fs::rename(join(old)?, join(new)?).map_err(|e| e.to_string()),
        FsOp::Delete { path, is_dir } => {
            let abs = join(path)?;
            if *is_dir { std::fs::remove_dir_all(&abs) } else { std::fs::remove_file(&abs) }.map_err(|e| e.to_string())
        },
    }
}

/// Ops dispatched during tests, in order — the fs fake.
#[cfg(test)]
pub(crate) static ISSUED: parking_lot::Mutex<Vec<FsOp>> = parking_lot::Mutex::new(Vec::new());
/// When set, the next `run` fails with this message — the failure-path fake.
#[cfg(test)]
pub(crate) static FAIL_WITH: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

/// Dispatch the op to disk. Under `cfg(test)` nothing is written: the op is
/// recorded and `FAIL_WITH` decides the result — the test workspace's
/// project root is the real repo, so the fake must not touch it.
#[cfg(not(test))]
fn run(root: &std::path::Path, op: &FsOp) -> Result<(), String> {
    perform(root, op)
}

#[cfg(test)]
fn run(_root: &std::path::Path, op: &FsOp) -> Result<(), String> {
    ISSUED.lock().push(op.clone());
    match FAIL_WITH.lock().take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Workspace {
    /// Arm the tree's inline input for a new file inside `dir` ("" = the
    /// project root). The dir expands so the input row is visible.
    pub fn begin_new_file(&mut self, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_explorer_edit(ExplorerEdit::NewFile { dir: dir.to_string() }, "", window, cx);
    }

    /// Same as `begin_new_file`, creating a directory.
    pub fn begin_new_folder(&mut self, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_explorer_edit(ExplorerEdit::NewFolder { dir: dir.to_string() }, "", window, cx);
    }

    /// Arm the inline input on `path`'s own row, seeded with the current
    /// name fully selected so typing replaces it.
    pub fn begin_rename_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        self.begin_explorer_edit(ExplorerEdit::Rename { path: path.to_string() }, &name, window, cx);
    }

    /// Shared arming: stash the edit target, seed the input, expand the
    /// parent for create edits, and focus the input next frame — the row
    /// only mounts after this render.
    fn begin_explorer_edit(&mut self, edit: ExplorerEdit, seed: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let ExplorerEdit::NewFile { dir } | ExplorerEdit::NewFolder { dir } = &edit
            && !dir.is_empty()
        {
            self.explorer.expanded.insert(dir.clone());
        }
        self.explorer.editing = Some(edit);
        self.explorer_input.update(cx, |s, cx| {
            s.set_value(seed.to_string(), window, cx);
            s.select_all(window, cx);
        });
        let input = self.explorer_input.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.notify();
    }

    /// Abandon the in-flight edit — Escape on the input. Focus returns to
    /// the composer so the hidden input doesn't keep it.
    pub fn cancel_explorer_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.explorer.editing.take().is_some() {
            self.composer.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }

    /// Enter or an outside click: validate the typed name, dispatch the op,
    /// and disarm. Empty input cancels silently (Finder-style); an invalid
    /// name keeps the edit armed — and the input focused — behind an error
    /// toast so it can be fixed.
    pub fn commit_explorer_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.explorer.editing.take() else { return };
        let typed = self.explorer_input.read(cx).value().trim().to_string();
        let Some(op) = edit_op(&edit, &typed) else {
            self.composer.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
            return;
        };
        if let Err(e) = op_paths_valid(&op) {
            self.explorer.editing = Some(edit);
            window.push_notification(Notification::error(e), cx);
            cx.notify();
            return;
        }
        self.composer.update(cx, |s, cx| s.focus(window, cx));
        self.run_fs_op(op, cx);
    }

    /// "Delete" on a file or dir row — a native confirm first, then the op.
    /// `rel` re-validates before dispatch so a stale or crafted path can't
    /// delete outside the project.
    pub fn delete_path(&mut self, rel: &str, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = relative_path(rel) else {
            window.push_notification(Notification::error(format!("Can't delete “{rel}” — not a project-relative path")), cx);
            return;
        };
        let rel = path.to_string_lossy().into_owned();
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        let what = if is_dir { "folder" } else { "file" };
        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Delete {what} “{name}”?"),
            Some(if is_dir {
                "The folder and everything in it will be deleted."
            } else {
                "The file will be deleted."
            }),
            &[PromptButton::ok("Delete"), PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.run_fs_op(FsOp::Delete { path: rel, is_dir }, cx));
        })
        .detach();
    }

    /// Run `op` on the background executor — a success re-scans the project
    /// so the tree picks up the change, a failure lands as an error toast.
    fn run_fs_op(&mut self, op: FsOp, cx: &mut Context<Self>) {
        let root = self.project.root().to_path_buf();
        let job = op.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { run(&root, &job) }).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.refresh_project_files(cx),
                Err(e) => window.push_notification(Notification::error(format!("{} failed: {e}", op.action())), cx),
            });
        })
        .detach();
    }
}

/// The op an edit commits to, or `None` for a silent cancel — empty input,
/// or a rename that lands on the same path. `typed` joins onto the edit's
/// parent dir so a nested name (`sub/file.rs`) works; `..`/absolute names
/// fail validation later in `commit_explorer_edit`.
fn edit_op(edit: &ExplorerEdit, typed: &str) -> Option<FsOp> {
    if typed.is_empty() {
        return None;
    }
    match edit {
        ExplorerEdit::NewFile { dir } => Some(FsOp::NewFile(joined(dir, typed))),
        ExplorerEdit::NewFolder { dir } => Some(FsOp::NewFolder(joined(dir, typed))),
        ExplorerEdit::Rename { path } => {
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            let rel = joined(parent, typed);
            (rel != *path).then(|| FsOp::Rename { old: path.clone(), new: rel })
        },
    }
}

/// `dir/name` with an empty `dir` collapsing to `name` — the project root.
fn joined(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

/// Every path in `op` must be project-relative — the pre-dispatch check
/// whose failure keeps the edit armed (the toast carries the reason).
fn op_paths_valid(op: &FsOp) -> Result<(), String> {
    let paths: &[&String] = match op {
        FsOp::NewFile(p) | FsOp::NewFolder(p) | FsOp::Delete { path: p, .. } => &[p],
        FsOp::Rename { old, new } => &[old, new],
    };
    for p in paths {
        if relative_path(p).is_none() {
            return Err(format!("“{p}” isn't a project-relative path"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "fs_ops_tests.rs"]
mod fs_ops_tests;
#[cfg(test)]
#[path = "fs_ops_ui_tests.rs"]
mod fs_ops_ui_tests;
