//! Branch ops for the Changes panel's branch header — rename/delete/list.
//! Split from `changes_ops.rs` for the SLOC cap.

use gpui_kit::*;

use crate::changes::GitOp;
use crate::workspace::Workspace;

impl Workspace {
    /// Arm the header's rename input for `name` — prefilled with the current
    /// name and focused so typing replaces it. Choosing "Rename…" from a
    /// picker's row menu dismisses the popover, so the input lives on the
    /// always-visible branch header.
    pub fn begin_rename_branch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.git.rename_target = Some(name.to_string());
        self.git.rename_input.update(cx, |s, cx| {
            s.set_value(name.to_string(), window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Disarm the rename input without running the op — the header's ✕.
    pub fn cancel_rename_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.git.rename_target = None;
        self.git.rename_input.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    /// `git branch -m` the armed `rename_target` to the rename input's value
    /// — Enter in the input and the header's ✓ share this path. An empty or
    /// unchanged name is refused before spawning, same as the commit box.
    pub fn rename_branch(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.git.rename_target.clone() else { return };
        let new = self.git.rename_input.read(cx).value().trim().to_string();
        if new.is_empty() || new == old {
            return;
        }
        self.run_git_op(GitOp::RenameBranch { old, new }, cx);
    }

    /// `git branch -d <name>` behind a native confirm — the picker's row
    /// menu. The current branch is never offered this item; the guard stays
    /// so a stale menu can't delete the checked-out branch. Git's own
    /// not-merged refusal lands as the note — no force delete.
    pub fn delete_branch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.git.branch.as_ref().is_some_and(|b| b.name == name) {
            return;
        }
        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Delete branch “{name}”?"),
            Some("Only merged branches can be deleted."),
            &[PromptButton::ok("Delete"), PromptButton::cancel("Cancel")],
            cx,
        );
        let name = name.to_string();
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.run_git_op(GitOp::DeleteBranch(name), cx));
        })
        .detach();
    }

    /// Re-list local branches for the picker — runs when the picker opens so
    /// branches created outside the app show up. Off the UI thread like the
    /// rest of the panel's git calls.
    pub fn refresh_branches(&mut self, cx: &mut Context<Self>) {
        self.git.branches_generation += 1;
        let generation = self.git.branches_generation;
        let dir = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let branches = cx.background_executor().spawn(async move { crate::git::list_branches(&dir) }).await;
            let _ = this.update(cx, |this, cx| this.land_branches(generation, branches, cx));
        })
        .detach();
    }

    /// Publish a fetched branch list — skipped when a newer fetch was
    /// requested while this one ran, same guard as `land_changes`.
    fn land_branches(&mut self, generation: u64, branches: Vec<crate::git::Branch>, cx: &mut Context<Self>) {
        if generation != self.git.branches_generation {
            return;
        }
        self.git.branches = branches;
        cx.notify();
    }
}
