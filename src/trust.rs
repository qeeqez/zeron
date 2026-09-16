//! Workspace trust — like VS Code's Workspace Trust and Codex's folder
//! prompt, a project folder isn't granted full agent access until the user
//! says so. `lifecycle::open_workspace_window_for` gates every opened folder
//! on `is_trusted`: an unlisted folder opens restricted (see
//! `Workspace::restrict_untrusted`) behind the trust dialog, and "Trust"
//! lands in `trust_project` — persisted under `Settings.trusted_folders` so
//! later opens of the same folder skip the prompt.

use std::path::{Path, PathBuf};

use gpui_kit::*;

use crate::backend::AccessMode;
use crate::workspace::Workspace;

/// Canonicalize for comparison and storage — `Project::open` already yields
/// canonical roots, but a hand-edited settings.json or a symlinked path
/// shouldn't slip past the check.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `root` is in the persisted trusted-folders list.
pub(crate) fn is_trusted(root: &Path) -> bool {
    let root = canonical(root);
    crate::persist::load_settings().trusted_folders.iter().any(|p| canonical(Path::new(p)) == root)
}

/// Add `root` to `Settings.trusted_folders` (canonicalized, deduped) and
/// persist — the "Trust" button's write path.
pub(crate) fn trust(root: &Path) {
    let root = canonical(root);
    let mut s = crate::persist::load_settings();
    if s.trusted_folders.iter().any(|p| canonical(Path::new(p)) == root) {
        return;
    }
    s.trusted_folders.push(root.to_string_lossy().into_owned());
    crate::persist::save_settings(&s);
}

impl Workspace {
    /// The access mode a turn actually gets — `Supervised` (read-only, every
    /// action asks) while the project folder is untrusted.
    pub(crate) fn effective_access(&self, access: AccessMode) -> AccessMode {
        if self.trusted { access } else { AccessMode::Supervised }
    }

    /// Mark this workspace's project untrusted: force every thread's access
    /// to `Supervised` until the folder is trusted. Called by
    /// `open_workspace_window_for` before the trust dialog opens — closing
    /// the dialog without trusting leaves this in place.
    pub(crate) fn restrict_untrusted(&mut self, cx: &mut Context<Self>) {
        self.trusted = false;
        self.access = AccessMode::Supervised;
        // Threads keep their own stamped access — clamp them too so a chat
        // saved with a permissive mode can't bypass the restriction.
        for chat in &mut self.chats {
            chat.access = Some(AccessMode::Supervised);
        }
        cx.notify();
    }

    /// Trust the project folder: persist it, lift the restriction and
    /// restore the configured access mode. Threads stamped while restricted
    /// revert to following the workspace selection.
    pub(crate) fn trust_project(&mut self, cx: &mut Context<Self>) {
        trust(self.project.root());
        self.trusted = true;
        self.access = AccessMode::from_name(&crate::persist::load_settings().access);
        for chat in &mut self.chats {
            chat.access = None;
        }
        self.stamp_thread();
        self.save();
        cx.notify();
    }
}
