//! Snapshot storage plumbing for `crate::snapshots`: describing a
//! checkpoint (size + how many files restoring it would touch) and deleting
//! its storage. Split from `snapshots.rs` to stay under the SLOC cap.

use std::path::Path;

use crate::checkpoints::Checkpoint;

/// (size, changed-file count) for one snapshot — see `SnapshotInfo`.
pub(crate) fn describe(workdir: &Path, checkpoint: &Checkpoint) -> (u64, Option<usize>) {
    match checkpoint {
        Checkpoint::Git(sha) => {
            let (bytes, names) = git_tree(workdir, sha);
            (bytes, names.and_then(|n| git_changed(workdir, sha, &n)))
        },
        Checkpoint::Copy(dir) => (dir_size(dir), copy_changed(dir, workdir)),
    }
}

/// (blob bytes, path → blob sha) of the snapshot commit — logical content
/// size, not pack size. `ls-tree -r -l` prints `<mode> blob <sha>
/// <size>\t<path>`. `None` when the commit is gone (gc'd) or the workdir
/// isn't a repo anymore.
fn git_tree(workdir: &Path, sha: &str) -> (u64, Option<std::collections::HashMap<String, String>>) {
    let Some(out) = crate::git::git(workdir, &["ls-tree", "-r", "-l", sha]) else { return (0, None) };
    let mut bytes = 0;
    let mut blobs = std::collections::HashMap::new();
    for line in out.lines() {
        let Some((meta, path)) = line.split_once('\t') else { continue };
        let mut fields = meta.split_whitespace();
        let (Some(_mode), Some(_kind), Some(blob), Some(size)) = (fields.next(), fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        bytes += size.parse::<u64>().unwrap_or(0);
        blobs.insert(path.to_string(), blob.to_string());
    }
    (bytes, Some(blobs))
}

/// Files `restore_git` would rewrite or remove: `diff --name-status` covers
/// worktree paths whose content differs from the snapshot; untracked files
/// the snapshot doesn't contain get removed by its `clean -fd`. HEAD never
/// enters the comparison — a file committed after the snapshot but matching
/// it on disk isn't "changed".
fn git_changed(workdir: &Path, sha: &str, snap: &std::collections::HashMap<String, String>) -> Option<usize> {
    let diff = crate::git::git(workdir, &["diff", "--name-status", sha, "--", "."])?;
    let others = crate::git::git(workdir, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    let mut count = others.lines().filter(|l| !l.is_empty() && !snap.contains_key(*l)).count();
    for line in diff.lines() {
        let Some((status, path)) = line.split_once('\t') else { continue };
        // A "deleted" path that exists on disk is an untracked file the
        // snapshot holds — restore only rewrites it when content differs,
        // so compare blob shas instead of trusting the D.
        if status == "D" && workdir.join(path).exists() && unchanged_blob(workdir, snap, path) {
            continue;
        }
        count += 1;
    }
    Some(count)
}

/// The on-disk file at `path` hashes to the same blob the snapshot holds.
fn unchanged_blob(workdir: &Path, snap: &std::collections::HashMap<String, String>, path: &str) -> bool {
    crate::git::git(workdir, &["hash-object", path]).is_some_and(|h| snap.get(path).is_some_and(|s| s == h.trim()))
}

/// File bytes under `dir` — `DirEntry::metadata` doesn't follow symlinks,
/// so links count as their own (tiny) size like the copy snapshot stores.
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| if e.path().is_dir() { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) })
        .sum()
}

/// Files `sync_dir` would copy in or remove to make `workdir` mirror `snap`.
/// `None` when the snapshot dir was deleted out from under the entry.
fn copy_changed(snap: &Path, workdir: &Path) -> Option<usize> {
    if !snap.is_dir() {
        return None;
    }
    Some(diff_count(snap, workdir) + extra_count(snap, workdir))
}

/// Entries under `snap` missing from `workdir` or differing in kind/content.
fn diff_count(snap: &Path, workdir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(snap) else { return 0 };
    entries
        .flatten()
        .map(|e| {
            let dst = workdir.join(e.file_name());
            let src_dir = e.path().is_dir();
            match std::fs::symlink_metadata(&dst) {
                Err(_) => 1,
                Ok(meta) if meta.is_dir() != src_dir => 1,
                Ok(_) if src_dir => diff_count(&e.path(), &dst),
                Ok(_) => usize::from(std::fs::read(e.path()).ok() != std::fs::read(&dst).ok()),
            }
        })
        .sum()
}

/// Entries under `workdir` that `snap` doesn't have — restore removes them.
/// A path present in both (even with a different kind) was already counted
/// by `diff_count`, so it's skipped here.
fn extra_count(snap: &Path, workdir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(workdir) else { return 0 };
    entries
        .flatten()
        .map(|e| {
            let src = snap.join(e.file_name());
            match std::fs::symlink_metadata(&src) {
                Ok(_) => 0,
                Err(_) if e.path().is_dir() => 1 + extra_count(&src, &e.path()),
                Err(_) => 1,
            }
        })
        .sum()
}

/// Drop a snapshot's storage: the keep-alive ref (then a best-effort gc so
/// the objects are actually freed) for git snapshots, the copy dir for
/// plain-directory ones. `workdir` is any directory inside the repo — the
/// project root works when the chat's own workdir is gone.
pub(crate) fn delete(workdir: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    match checkpoint {
        Checkpoint::Git(sha) => {
            crate::git::git_env(workdir, &["update-ref", "-d", &format!("refs/rixl/checkpoints/{sha}")], &[])?;
            // Without gc the commit lingers as loose objects; --prune=now
            // frees them immediately. Failure is tolerable — the next
            // auto-gc collects them.
            let _ = crate::git::git_env(workdir, &["gc", "--prune=now", "--quiet"], &[]);
            Ok(())
        },
        Checkpoint::Copy(dir) => std::fs::remove_dir_all(dir).map_err(|e| format!("remove {}: {e}", dir.display())),
    }
}
