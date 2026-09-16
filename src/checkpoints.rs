//! Per-turn checkpoints — snapshot the thread's working directory before
//! each backend turn so a completed turn can be reverted. Git workdirs
//! snapshot into a detached commit-tree (kept alive by a ref under
//! `refs/rixl/checkpoints/`); plain directories fall back to a file copy
//! under the project's store dir. Either way the user's index, HEAD and
//! staged state are never touched — only worktree file contents move on
//! restore.

use std::path::{Path, PathBuf};

/// A recorded checkpoint — the workdir state a turn started from.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Checkpoint {
    /// Detached commit-tree sha in the workdir's repository.
    Git(String),
    /// Full directory copy at this absolute path (non-git workdirs).
    Copy(PathBuf),
}

/// A checkpoint pinned to the user message that opened its turn. `at`
/// duplicates the message timestamp so a stale entry — an index reused
/// after `/clear` or an edit — can't restore the wrong state.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TurnCheckpoint {
    pub ix: usize,
    pub at: std::time::SystemTime,
    pub checkpoint: Checkpoint,
}

/// Where non-git snapshots live: the project's store dir, outside the
/// snapshotted tree so a copy never contains itself.
pub(crate) fn store_dir(project: &crate::project::Project) -> PathBuf {
    project.dir().join("checkpoints")
}

/// The checkpoint recorded for the turn that message `ix` opened — the
/// index alone isn't enough (see `TurnCheckpoint::at`).
pub(crate) fn for_message(chat: &crate::model::Chat, ix: usize) -> Option<&TurnCheckpoint> {
    let msg = chat.messages.get(ix)?;
    chat.checkpoints.iter().rev().find(|c| c.ix == ix && c.at == msg.at)
}

/// Snapshot `workdir` before a turn. `id` names the copy snapshot
/// (`chat<message-ix>`); git snapshots ignore it — the commit sha is the
/// name. `None` when the workdir is missing or unreadable.
pub(crate) fn snapshot(workdir: &Path, store: &Path, id: &str) -> Option<Checkpoint> {
    if !workdir.is_dir() {
        return None;
    }
    if is_git_repo(workdir) {
        snapshot_git(workdir).ok().map(Checkpoint::Git)
    } else {
        snapshot_copy(workdir, &store.join(id)).ok().map(Checkpoint::Copy)
    }
}

/// Restore `workdir` to `checkpoint`. Errors surface to the user as a
/// chat note — a gc'd commit or a deleted snapshot dir both land here.
pub(crate) fn restore(workdir: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    match checkpoint {
        Checkpoint::Git(sha) => restore_git(workdir, sha),
        Checkpoint::Copy(dir) => sync_dir(dir, workdir),
    }
}

fn is_git_repo(dir: &Path) -> bool {
    crate::git::git(dir, &["rev-parse", "--git-dir"]).is_some()
}

/// Commit-tree identity — checkpoint commits carry no user identity, so
/// they work in repos without `user.name`/`user.email` configured.
const IDENT: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Rixl"),
    ("GIT_AUTHOR_EMAIL", "rixl@localhost"),
    ("GIT_COMMITTER_NAME", "Rixl"),
    ("GIT_COMMITTER_EMAIL", "rixl@localhost"),
];

/// Scratch index path — unique per call so concurrent turns never share.
pub(crate) fn temp_index() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!("rixl-ckpt-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
}

/// Snapshot every non-ignored file under `workdir` into a detached commit.
/// A scratch index staged from empty captures the whole tree — tracked,
/// modified, staged and untracked alike — without touching the real index.
fn snapshot_git(workdir: &Path) -> Result<String, String> {
    let index = temp_index();
    let index_str = index.to_string_lossy().into_owned();
    let scratch = [("GIT_INDEX_FILE", index_str.as_str())];
    let result = (|| {
        let git = |args: &[&str], envs: &[(&str, &str)]| crate::git::git_env(workdir, args, envs);
        git(&["read-tree", "--empty"], &scratch)?;
        git(&["add", "-A"], &scratch)?;
        let tree = git(&["write-tree"], &scratch)?;
        let mut env = scratch.to_vec();
        env.extend_from_slice(&IDENT);
        let commit = git(&["commit-tree", tree.trim(), "-m", "rixl checkpoint"], &env)?;
        let sha = commit.trim().to_string();
        // The ref keeps the commit reachable — without it `git gc` could
        // prune the snapshot before a later revert needs it.
        git(&["update-ref", &format!("refs/rixl/checkpoints/{sha}"), &sha], &[])?;
        Ok(sha)
    })();
    let _ = std::fs::remove_file(&index);
    result
}

/// Restore the worktree files under `workdir` to the snapshot commit.
/// `restore --worktree` rewrites tracked content (recreating deleted
/// files); `clean -fd` under a scratch index holding the snapshot removes
/// files the turn added. The real index is untouched — staged work stays
/// staged even when its worktree content reverts.
fn restore_git(workdir: &Path, sha: &str) -> Result<(), String> {
    let index = temp_index();
    let index_str = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let result = (|| {
        crate::git::git_env(workdir, &["restore", &format!("--source={sha}"), "--worktree", "--", "."], &[])?;
        crate::git::git_env(workdir, &["read-tree", sha], &env)?;
        crate::git::git_env(workdir, &["clean", "-fd", "--", "."], &env)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&index);
    result
}

/// Copy `workdir` into `dst`, replacing any previous snapshot there.
fn snapshot_copy(workdir: &Path, dst: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dst);
    copy_dir(workdir, dst)?;
    Ok(dst.to_path_buf())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_symlink() {
            copy_link(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to).map_err(|e| format!("copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Recreate a symlink; falls back to copying the target's content on
/// platforms without `symlink` (the snapshot still restores the bytes).
fn copy_link(from: &Path, to: &Path) -> Result<(), String> {
    let target = std::fs::read_link(from).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, to).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(&target, to).map_err(|e| e.to_string())
    }
}

/// Make `dst` mirror `src`: copy everything in, then remove what the
/// snapshot doesn't contain. `dst` may not exist yet — a workdir deleted
/// since the snapshot is recreated.
fn sync_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Err(format!("checkpoint {} is gone", src.display()));
    }
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    remove_extras(src, dst)?;
    copy_dir(src, dst)
}

/// Delete entries in `dst` that `src` doesn't have — including entries
/// whose kind changed (file ↔ dir), which `copy_dir` can't overwrite.
fn remove_extras(src: &Path, dst: &Path) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(dst) else { return Ok(()) };
    for entry in entries.flatten() {
        let keep = src.join(entry.file_name());
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let stale = match std::fs::symlink_metadata(&keep) {
            Err(_) => true,
            Ok(meta) => meta.is_dir() != ty.is_dir(),
        };
        if stale {
            let path = entry.path();
            if ty.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

impl crate::workspace::Workspace {
    /// Snapshot the workdir before a backend turn starts and pin it to the
    /// user message that opened the turn — retry pops the reply, so the
    /// last user message is always the turn's first. No-op when the chat
    /// has no user message yet or the workdir can't be snapshotted.
    pub(crate) fn record_turn_checkpoint(&mut self, chat_id: u64, workdir: &Path) {
        let turn = self.chats[self.active]
            .messages
            .iter()
            .rposition(|m| m.role == crate::model::Role::User)
            .and_then(|ix| self.chats[self.active].messages.get(ix).map(|m| (ix, m.at)));
        if let Some((ix, at)) = turn
            && let Some(checkpoint) = snapshot(workdir, &store_dir(&self.project), &format!("chat{chat_id}-{ix}"))
            && let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id)
        {
            chat.checkpoints.push(TurnCheckpoint { ix, at, checkpoint });
        }
    }
}
