//! Working-tree git changes for the Changes panel — collected by shelling out
//! to `git` in the project root. Parsing lives in `parse_status`/`parse_numstat`
//! so tests can feed fixture output without a real repository. Line-level diffs
//! for expanded rows live in `crate::changes_diff`.

/// One file's working-tree change, as listed by `git status --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Source path of a rename/copy — porcelain `-z` emits it as a second
    /// field; `None` for every other change kind. `diff HEAD` needs both
    /// names to show the rename delta instead of an all-added new file.
    pub source: Option<String>,
    pub status: ChangeStatus,
    pub added: u32,
    pub deleted: u32,
    /// Parsed unified diff, loaded on demand when the panel row expands —
    /// `None` means collapsed. Collection never fills this; the workspace
    /// owns no extra per-file state, so the row doubles as the cache.
    pub diff: Option<crate::changes_diff::FileDiff>,
    /// Token of the in-flight diff load, 0 when none. Collapse clears it so
    /// a result that lands late is discarded instead of reopening the row;
    /// a fresh token per expand keeps an older load from attaching after a
    /// collapse+re-expand. UI-only — collection leaves it 0.
    pub diff_load: u64,
}

/// Display status derived from the two-column porcelain code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflicted,
}

/// The repo's empty-tree object — the diff base when HEAD is unborn (a fresh
/// repo with no commits), giving the same net worktree delta. Derived via
/// `hash-object` because the well-known SHA-1 id is invalid in SHA-256 repos.
pub(crate) fn empty_tree_id(dir: &std::path::Path) -> Option<String> {
    git(dir, &["hash-object", "-t", "tree", "/dev/null"]).map(|id| id.trim().to_string())
}

/// Collect changes under `dir`: porcelain status for the file list, numstat
/// for line counts, filesystem line count for untracked.
pub(crate) fn collect(dir: &std::path::Path) -> Vec<FileChange> {
    let Some(status) = git(dir, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]) else {
        return Vec::new();
    };
    let mut changes = parse_status(&status);
    if changes.is_empty() {
        return changes;
    }
    // Net HEAD→worktree counts: `diff HEAD` covers staged + unstaged in one
    // pass, so a partially-staged file reports its combined delta rather than
    // whichever half was merged last. On an unborn HEAD there is nothing to
    // diff against — fall back to the empty tree.
    let numstat = git(dir, &["diff", "HEAD", "--numstat", "-z"])
        .or_else(|| git(dir, &["diff", &empty_tree_id(dir)?, "--numstat", "-z"]))
        .unwrap_or_default();
    let counts = parse_numstat(&numstat);
    for change in &mut changes {
        if let Some((added, deleted)) = counts.get(&change.path) {
            change.added = *added;
            change.deleted = *deleted;
        } else if change.status == ChangeStatus::Added {
            // Untracked files never appear in numstat — count lines on disk.
            change.added = line_count(&dir.join(&change.path));
        }
    }
    changes
}

/// Run `git` in `dir`; stdout on success, `None` on spawn failure, non-zero
/// exit (not a repo), or non-UTF-8 output.
pub(crate) fn git(dir: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `git` in `dir` with extra environment variables; stdout on success,
/// stderr text on failure. Checkpoint plumbing needs this over `git()`:
/// `GIT_INDEX_FILE` redirects index reads/writes to a scratch file so the
/// user's real index is never touched, and `commit-tree` needs a synthetic
/// identity when the repo has none configured.
pub(crate) fn git_env(dir: &std::path::Path, args: &[&str], envs: &[(&str, &str)]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs(envs.iter().copied())
        .output()
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `git diff` variant with bounded output: reads at most `max_bytes` of
/// stdout, then kills the child rather than buffering an unbounded diff.
/// Returns `(output, hit_cap)`. `None` on spawn failure or an exit code
/// other than 0/1 (1 = differences found, always for `--no-index`) — unless
/// the cap was hit, where the partial output is still the payload.
pub(crate) fn git_diff(dir: &std::path::Path, args: &[&str], max_bytes: u64) -> Option<(String, bool)> {
    use std::io::Read;
    let mut child = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return None;
    };
    let mut buf = Vec::new();
    if stdout.take(max_bytes + 1).read_to_end(&mut buf).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let capped = buf.len() as u64 > max_bytes;
    buf.truncate(max_bytes as usize);
    if capped {
        // The child is likely still writing — kill it instead of waiting on
        // a full pipe.
        let _ = child.kill();
    }
    let code = child.wait().ok()?.code().unwrap_or(-1);
    (capped || code == 0 || code == 1).then(|| (String::from_utf8_lossy(&buf).into_owned(), capped))
}

/// Parse `git status --porcelain=v1 -z` output. Entries are NUL-separated
/// `XY path`; renames/copies append a second field holding the source path.
pub(crate) fn parse_status(raw: &str) -> Vec<FileChange> {
    let mut fields = raw.split('\0');
    let mut out = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let (code, path) = (&field.as_bytes()[..2], &field[3..]);
        // `!!` ignored entries only appear with --ignored; never a change.
        if code == b"!!" {
            continue;
        }
        let status = status_of(code);
        // Renames/copies append the source path as a second NUL field —
        // consume it whenever the raw code says so, even when the display
        // status masks it (`RD` shows Deleted but still emits the field).
        let source = if code.contains(&b'R') || code.contains(&b'C') {
            fields.next().map(str::to_string).filter(|s| !s.is_empty())
        } else {
            None
        };
        out.push(FileChange {
            path: path.to_string(),
            source,
            status,
            added: 0,
            deleted: 0,
            diff: None,
            diff_load: 0,
        });
    }
    out
}

/// Map the two-column porcelain code to a display status. Conflict wins over
/// delete, delete over rename, rename over add, add over modify — the most
/// surprising state is the one worth showing.
fn status_of(code: &[u8]) -> ChangeStatus {
    if code.contains(&b'U') || code == b"AA" || code == b"DD" {
        ChangeStatus::Conflicted
    } else if code.contains(&b'D') {
        ChangeStatus::Deleted
    } else if code.contains(&b'R') || code.contains(&b'C') {
        ChangeStatus::Renamed
    } else if code.contains(&b'A') || code == b"??" {
        ChangeStatus::Added
    } else {
        ChangeStatus::Modified
    }
}

/// Parse `git diff --numstat -z` output into `path → (added, deleted)`. Binary
/// files report `-` counts and parse as zero. A rename's path field is empty;
/// the following two fields are the old then new path, keyed under the new.
pub(crate) fn parse_numstat(raw: &str) -> std::collections::HashMap<String, (u32, u32)> {
    let mut fields = raw.split('\0');
    let mut out = std::collections::HashMap::new();
    while let Some(field) = fields.next() {
        let mut cols = field.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            // Rename: skip the old path, take the new one.
            let _old = fields.next();
            fields.next().unwrap_or_default()
        } else {
            path
        };
        if path.is_empty() {
            continue;
        }
        out.insert(path.to_string(), (parse_num(added), parse_num(deleted)));
    }
    out
}

fn parse_num(s: &str) -> u32 {
    s.parse().unwrap_or(0)
}

/// Line count for an untracked file — newlines plus a trailing partial line.
/// Binary files (NUL in the first 8 KiB) and unreadable files report zero.
fn line_count(path: &std::path::Path) -> u32 {
    const MAX: u64 = 8 * 1024 * 1024;
    if std::fs::metadata(path).map(|m| m.len() > MAX).unwrap_or(true) {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else { return 0 };
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    newlines + u32::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'))
}
