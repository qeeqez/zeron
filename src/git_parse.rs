//! Parsers for `git` command output — sibling to `git.rs` so both stay under
//! the SLOC cap. Everything here is pure: tests feed fixture output without a
//! real repository.

use crate::git::{ChangeStatus, FileChange};

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
            staged: staged_of(code),
            added: 0,
            deleted: 0,
            diff: None,
            diff_load: 0,
        });
    }
    out
}

/// Whether the porcelain `X` column records an index change — the file is
/// (partially) staged. ` `/`?`/`!` mean the index is untouched; unmerged
/// entries (`U`, `AA`, `DD`) count as unstaged so the panel's stage toggle
/// offers `git add` to mark them resolved.
fn staged_of(code: &[u8]) -> bool {
    !matches!(code[0], b' ' | b'?' | b'!') && !unmerged(code)
}

/// Unmerged (conflicted) porcelain codes: any `U`, or the `AA`/`DD` pairs
/// where both sides added or deleted.
fn unmerged(code: &[u8]) -> bool {
    code.contains(&b'U') || code == b"AA" || code == b"DD"
}

/// Map the two-column porcelain code to a display status. Conflict wins over
/// delete, delete over rename, rename over add, add over modify — the most
/// surprising state is the one worth showing.
fn status_of(code: &[u8]) -> ChangeStatus {
    if unmerged(code) {
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
pub(crate) fn line_count(path: &std::path::Path) -> u32 {
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
