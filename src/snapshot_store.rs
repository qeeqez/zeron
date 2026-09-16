//! Snapshot storage plumbing for `crate::snapshots`: describing a
//! checkpoint (size + the files restoring it would touch) and deleting its
//! storage. Split from `snapshots.rs` to stay under the SLOC cap.

use std::path::Path;

use crate::checkpoints::Checkpoint;

/// How a file restore would touch differs between the workdir and the
/// checkpoint — the same names `git diff --name-status` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotStatus {
    /// In the workdir but not the checkpoint — restore deletes it.
    Added,
    /// In both with different content (or a file ↔ dir kind change).
    Modified,
    /// In the checkpoint but not the workdir — restore recreates it.
    Deleted,
}

/// One path `restore` would rewrite or remove, relative to the workdir.
/// Directory entries carry a trailing `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotFile {
    pub path: String,
    pub status: SnapshotStatus,
}

/// (size, changed-file count) for one snapshot — see `SnapshotInfo`.
pub(crate) fn describe(workdir: &Path, checkpoint: &Checkpoint) -> (u64, Option<usize>) {
    let (bytes, files) = describe_files(workdir, checkpoint);
    (bytes, files.map(|f| f.len()))
}

/// (size, paths restore would change) — the row's expanded list and the
/// restore confirm's names. `None` for the files when the diff can't be
/// computed (a gc'd commit, a deleted copy dir, a missing workdir).
pub(crate) fn describe_files(workdir: &Path, checkpoint: &Checkpoint) -> (u64, Option<Vec<SnapshotFile>>) {
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
/// it on disk isn't "changed". A rename reports as its two paths: the old
/// name recreated, the new one removed.
fn git_changed(workdir: &Path, sha: &str, snap: &std::collections::HashMap<String, String>) -> Option<Vec<SnapshotFile>> {
    let diff = crate::git::git(workdir, &["diff", "--name-status", sha, "--", "."])?;
    let others = crate::git::git(workdir, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    let mut files: Vec<SnapshotFile> = others
        .lines()
        .filter(|l| !l.is_empty() && !snap.contains_key(*l))
        .map(|path| SnapshotFile { path: path.to_string(), status: SnapshotStatus::Added })
        .collect();
    for line in diff.lines() {
        let Some((status, path)) = line.split_once('\t') else { continue };
        let Some(&code) = status.as_bytes().first() else { continue };
        match code {
            b'A' => files.push(SnapshotFile { path: path.to_string(), status: SnapshotStatus::Added }),
            b'M' | b'T' => files.push(SnapshotFile { path: path.to_string(), status: SnapshotStatus::Modified }),
            b'D' => {
                // A "deleted" path that exists on disk is an untracked file
                // the snapshot holds — restore only rewrites it when content
                // differs, so compare blob shas instead of trusting the D.
                if workdir.join(path).exists() && unchanged_blob(workdir, snap, path) {
                    continue;
                }
                files.push(SnapshotFile { path: path.to_string(), status: SnapshotStatus::Deleted });
            },
            b'R' | b'C' => {
                // `R100\told\tnew` — restore recreates the old name and
                // removes the new one.
                let (old, new) = path.split_once('\t').map_or((path, None), |(o, n)| (o, Some(n)));
                files.push(SnapshotFile { path: old.to_string(), status: SnapshotStatus::Deleted });
                if let Some(new) = new {
                    files.push(SnapshotFile { path: new.to_string(), status: SnapshotStatus::Added });
                }
            },
            _ => files.push(SnapshotFile { path: path.to_string(), status: SnapshotStatus::Modified }),
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Some(files)
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
fn copy_changed(snap: &Path, workdir: &Path) -> Option<Vec<SnapshotFile>> {
    if !snap.is_dir() {
        return None;
    }
    let mut files = Vec::new();
    diff_entries(snap, workdir, "", &mut files);
    extra_entries(snap, workdir, "", &mut files);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Some(files)
}

/// Entries under `snap` missing from `workdir` or differing in kind/content.
/// A missing directory lists as one `dir/` entry — restore recreates the
/// whole subtree — while a differing directory recurses to the files inside.
fn diff_entries(snap: &Path, workdir: &Path, prefix: &str, out: &mut Vec<SnapshotFile>) {
    let Ok(entries) = std::fs::read_dir(snap) else { return };
    for e in entries.flatten() {
        let dst = workdir.join(e.file_name());
        let path = format!("{prefix}{}", e.file_name().to_string_lossy());
        let src_dir = e.path().is_dir();
        match std::fs::symlink_metadata(&dst) {
            Err(_) => out.push(SnapshotFile {
                path: if src_dir { format!("{path}/") } else { path },
                status: SnapshotStatus::Deleted,
            }),
            Ok(meta) if meta.is_dir() != src_dir => out.push(SnapshotFile { path, status: SnapshotStatus::Modified }),
            Ok(_) if src_dir => diff_entries(&e.path(), &dst, &format!("{path}/"), out),
            Ok(_) if std::fs::read(e.path()).ok() != std::fs::read(&dst).ok() => {
                out.push(SnapshotFile { path, status: SnapshotStatus::Modified });
            },
            Ok(_) => {},
        }
    }
}

/// Entries under `workdir` that `snap` doesn't have — restore removes them.
/// A path present in both (even with a different kind) was already listed
/// by `diff_entries`, so it's skipped here.
fn extra_entries(snap: &Path, workdir: &Path, prefix: &str, out: &mut Vec<SnapshotFile>) {
    let Ok(entries) = std::fs::read_dir(workdir) else { return };
    for e in entries.flatten() {
        let src = snap.join(e.file_name());
        let path = format!("{prefix}{}", e.file_name().to_string_lossy());
        match std::fs::symlink_metadata(&src) {
            Ok(_) => {},
            Err(_) if e.path().is_dir() => {
                out.push(SnapshotFile { path: format!("{path}/"), status: SnapshotStatus::Added });
                extra_entries(&src, &e.path(), &format!("{path}/"), out);
            },
            Err(_) => out.push(SnapshotFile { path, status: SnapshotStatus::Added }),
        }
    }
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
