//! Merge-back for per-thread worktrees — the "Merge into project" action.
//! `merge_into` diffs the worktree against the merge-base of its HEAD and
//! the project root's HEAD (the commit `worktree add --detach` started
//! from), so committed and uncommitted work both land; a scratch index
//! staged from the worktree pulls untracked files into the patch without
//! touching the real index. The patch applies to the project checkout via
//! `git apply`, falling back to `--3way` when plain context doesn't match.
//! Conflicts surface in an activity note naming the failed paths and the
//! worktree stays; a clean merge refreshes the Changes panel and offers
//! worktree removal when nothing uncommitted remains.

use std::path::{Path, PathBuf};

use gpui_kit::*;

use crate::workspace::Workspace;

/// What `merge_into` did with the worktree's delta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MergeOutcome {
    /// The patch applied to the project checkout (plain or 3-way).
    Applied,
    /// No delta — the worktree matches the merge-base.
    Empty,
    /// `git apply --3way` left conflict markers; the paths it names.
    Conflicted(Vec<String>),
    /// The merge couldn't run or apply — git's stderr for the note.
    Failed(String),
}

/// Apply the worktree `wt`'s delta to the project checkout `root`. The
/// worktree is only read — a failed merge leaves it exactly as it was.
pub(crate) fn merge_into(root: &Path, wt: &Path) -> MergeOutcome {
    try_merge(root, wt).unwrap_or_else(MergeOutcome::Failed)
}

fn try_merge(root: &Path, wt: &Path) -> Result<MergeOutcome, String> {
    let base = merge_base(root, wt)?;
    let patch = worktree_patch(wt, &base)?;
    if patch.trim().is_empty() {
        return Ok(MergeOutcome::Empty);
    }
    Ok(apply_patch(root, &patch))
}

/// The commit the worktree diverged from — `merge-base` of the worktree's
/// HEAD and the project checkout's current HEAD. Both live in the same
/// object store, so either side can run the lookup; the worktree does so
/// the base is found even when the root's HEAD moved since creation.
pub(super) fn merge_base(root: &Path, wt: &Path) -> Result<String, String> {
    let head = super::git_err(root, &["rev-parse", "HEAD"])?;
    super::git_err(wt, &["merge-base", "HEAD", head.trim()]).map(|s| s.trim().to_string())
}

/// The worktree's full delta against `base` as one patch: a scratch index
/// is read from `base` then `add -A`'d to the worktree's files, so
/// `diff --cached` covers committed, staged, unstaged and untracked work
/// alike. The real index is never touched — same trick as checkpoints.
fn worktree_patch(wt: &Path, base: &str) -> Result<String, String> {
    let index = crate::checkpoints::temp_index();
    let index_str = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let result = (|| {
        crate::git::git_env(wt, &["read-tree", base], &env)?;
        crate::git::git_env(wt, &["add", "-A"], &env)?;
        crate::git::git_env(wt, &["diff", "--cached", "--binary", base], &env)
    })();
    let _ = std::fs::remove_file(&index);
    result
}

/// Apply `patch` to `root`. Plain `git apply` first — it's atomic, so a
/// context mismatch changes nothing — then `git apply --3way`, which
/// merges against the patch's blob ids and leaves `<<<<<<<` markers plus
/// unmerged index entries the Changes panel's conflicts section resolves.
fn apply_patch(root: &Path, patch: &str) -> MergeOutcome {
    if crate::git::git_stdin(root, &["apply"], patch).is_ok() {
        return MergeOutcome::Applied;
    }
    match crate::git::git_stdin(root, &["apply", "--3way"], patch) {
        Ok(_) => MergeOutcome::Applied,
        Err(e) => {
            let paths = conflicted_paths(&e);
            if paths.is_empty() { MergeOutcome::Failed(e) } else { MergeOutcome::Conflicted(paths) }
        },
    }
}

/// The paths `git apply --3way` failed on, parsed from its stderr:
/// "Applied patch to '<path>' with conflicts." for merged-with-markers
/// files, "error: <path>: <reason>" for the rest ("does not match index",
/// "already exists", "patch does not apply"). Other output lines carry no
/// path. Deduped, order preserved.
fn conflicted_paths(stderr: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let path = if let Some(rest) = line.strip_prefix("Applied patch to '") {
            rest.split('\'').next()
        } else {
            line.strip_prefix("error: ").and_then(error_path)
        };
        if let Some(path) = path
            && !path.is_empty()
            && !paths.iter().any(|p| p == path)
        {
            paths.push(path.to_string());
        }
    }
    paths
}

/// The path in an "error: …" line — after "patch failed: " it's
/// "<path>:<line>" (the numeric tail drops); otherwise "<path>: <reason>"
/// splits at the last colon so names containing ": " still truncate at
/// the reason. `None` when the line names no path.
fn error_path(rest: &str) -> Option<&str> {
    let rest = rest.strip_prefix("patch failed: ").unwrap_or(rest);
    match rest.rsplit_once(':') {
        Some((p, n)) if rest.contains(": ") || n.chars().all(|c| c.is_ascii_digit()) => Some(p),
        _ => None,
    }
}

/// The activity entry for a merge outcome — `chat_created` links the row
/// back to the chat; a deleted chat leaves the click inert, same as the
/// kept-worktrees note.
fn merge_entry(job: &MergeJob, body: String) -> crate::activity::ActivityEntry {
    crate::activity::ActivityEntry {
        kind: crate::activity::ActivityKind::Note,
        chat_title: job.title.clone(),
        body,
        chat_created: job.created_at,
        at: std::time::SystemTime::now(),
        unread: true,
    }
}

/// Everything `land_worktree_merge` needs about the chat that asked for
/// the merge — bundled to stay under the arg-count lint.
struct MergeJob {
    chat_id: u64,
    /// Snapshot for the activity entry — a mid-merge delete must not
    /// rewrite or lose the record.
    title: SharedString,
    created_at: std::time::SystemTime,
    workdir: PathBuf,
}

impl Workspace {
    /// "Merge into project" — apply the active chat's worktree delta to the
    /// project checkout. Refused while the chat's turn runs (its files are
    /// still moving) or a Changes-panel op holds `git.busy` (the apply
    /// would race its index writes).
    pub(crate) fn merge_worktree_into_project(&mut self, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        if !chat.worktree || chat.running || self.git.busy {
            return;
        }
        let root = self.project.root().to_path_buf();
        let wt = PathBuf::from(&chat.workdir);
        if !wt.starts_with(self.project.worktrees_dir()) || !wt.is_dir() {
            return;
        }
        let job = MergeJob {
            chat_id: chat.id,
            title: chat.title.clone(),
            created_at: chat.created_at,
            workdir: wt,
        };
        self.git.busy = true;
        cx.spawn(async move |this, cx| {
            let dir = job.workdir.clone();
            let outcome = cx.background_executor().spawn(async move { merge_into(&root, &dir) }).await;
            let _ = this.update_in(cx, |this, window, cx| this.land_worktree_merge(job, outcome, window, cx));
        })
        .detach();
        cx.notify();
    }

    /// Publish the merge outcome: refresh the Changes panel so the applied
    /// files (or the conflict rows) show, then note the result — conflicts
    /// also land in the activity feed with their paths. A clean merge with
    /// nothing left in the worktree offers to remove the checkout.
    fn land_worktree_merge(&mut self, job: MergeJob, outcome: MergeOutcome, window: &mut Window, cx: &mut Context<Self>) {
        self.git.busy = false;
        self.refresh_changes(cx);
        match outcome {
            MergeOutcome::Applied | MergeOutcome::Empty => self.land_clean_merge(job, outcome == MergeOutcome::Applied, window, cx),
            MergeOutcome::Conflicted(paths) => self.land_merge_conflicts(&job, &paths, cx),
            MergeOutcome::Failed(e) => self.land_merge_failure(&job, &e, cx),
        }
        cx.notify();
    }

    /// Applied/empty merges: offer removal when the worktree is clean,
    /// else note on the chat why the checkout stays.
    fn land_clean_merge(&mut self, job: MergeJob, applied: bool, window: &mut Window, cx: &mut Context<Self>) {
        if super::is_clean(&job.workdir) {
            self.offer_worktree_removal(job, window, cx);
            return;
        }
        if self.chat_index(job.chat_id).is_none() {
            return;
        }
        let note = if applied {
            "Merged into the project checkout — the worktree still holds uncommitted files (ignored or generated), so it stays."
        } else {
            "Nothing to merge — the worktree matches the project."
        };
        self.note_in(job.chat_id, note.to_string(), cx);
    }

    /// Conflicted merges: the failed paths go to the activity feed (the
    /// required record) plus a chat note while the chat still exists.
    fn land_merge_conflicts(&mut self, job: &MergeJob, paths: &[String], cx: &mut Context<Self>) {
        let body = format!(
            "Merge into project hit conflicts in: {}. Resolve them in the Changes panel — the worktree stays as it was.",
            paths.join(", ")
        );
        self.push_activity(merge_entry(job, body.clone()));
        if self.chat_index(job.chat_id).is_some() {
            self.note_in(job.chat_id, format!("**Merge conflicts** — {body}"), cx);
        }
    }

    /// A merge that couldn't run or apply at all — a chat note while the
    /// chat lives, an activity entry when it's gone.
    fn land_merge_failure(&mut self, job: &MergeJob, e: &str, cx: &mut Context<Self>) {
        if self.chat_index(job.chat_id).is_some() {
            self.note_in(job.chat_id, format!("**Merge into project failed:** {e}"), cx);
        } else {
            self.push_activity(merge_entry(job, format!("Merge into project failed: {e}")));
        }
    }

    /// The post-merge prompt: the worktree's delta is in the project and
    /// the checkout is clean, so offer to drop it. Confirming clears the
    /// chat's worktree flags — its turns continue in the project root.
    fn offer_worktree_removal(&mut self, job: MergeJob, window: &mut Window, cx: &mut Context<Self>) {
        let name = job
            .workdir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| job.workdir.display().to_string());
        let rx = window.prompt(
            PromptLevel::Info,
            &format!("Remove worktree “{name}”?"),
            Some("Its changes are merged into the project checkout."),
            &[PromptButton::ok("Remove"), PromptButton::cancel("Keep")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.remove_merged_worktree(job, cx));
        })
        .detach();
    }

    /// The confirmed half of `offer_worktree_removal`: re-check the chat
    /// still owns the dir, then remove it and point the chat back at the
    /// project root so the badge and ⋯ menu stop claiming a worktree.
    fn remove_merged_worktree(&mut self, job: MergeJob, cx: &mut Context<Self>) {
        let Some(ix) = self.chat_index(job.chat_id) else { return };
        if Path::new(&self.chats[ix].workdir) != job.workdir {
            return;
        }
        match super::remove(self.project.root(), &job.workdir) {
            super::Removal::Removed => {
                let chat = &mut self.chats[ix];
                chat.worktree = false;
                chat.workdir = self.project.root().to_string_lossy().into_owned();
                self.save();
            },
            super::Removal::Kept(reason) => {
                self.note_in(job.chat_id, format!("**Worktree kept** — {reason}."), cx);
            },
        }
        cx.notify();
    }
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "worktree_merge_tests.rs"]
mod worktree_merge_tests;
