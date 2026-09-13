//! Working-tree git changes for the Changes panel — collected by shelling out
//! to `git` in the app cwd. Parsing lives in `parse_status`/`parse_numstat` so
//! tests can feed fixture output without a real repository.

/// One file's working-tree change, as listed by `git status --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub status: ChangeStatus,
    pub added: u32,
    pub deleted: u32,
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

/// Collect the working-tree changes under the process cwd. Empty when the cwd
/// is not a repo or `git` is unavailable.
pub(crate) fn collect_in_cwd() -> Vec<FileChange> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    collect(&cwd)
}

/// Collect changes under `dir`: porcelain status for the file list, numstat
/// (unstaged + staged) for line counts, filesystem line count for untracked.
pub(crate) fn collect(dir: &std::path::Path) -> Vec<FileChange> {
    let Some(status) = git(dir, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]) else {
        return Vec::new();
    };
    let mut changes = parse_status(&status);
    if changes.is_empty() {
        return changes;
    }
    let mut counts = parse_numstat(&git(dir, &["diff", "--numstat", "-z"]).unwrap_or_default());
    counts.extend(parse_numstat(&git(dir, &["diff", "--cached", "--numstat", "-z"]).unwrap_or_default()));
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
fn git(dir: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
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
        if status == ChangeStatus::Renamed {
            // Consume the source-path field; the entry path is the new name.
            fields.next();
        }
        out.push(FileChange { path: path.to_string(), status, added: 0, deleted: 0 });
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
