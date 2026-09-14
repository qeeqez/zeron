//! Line-level diffs for the Changes panel. `diff_for_file` shells out to git
//! for one changed path; `parse_diff` turns unified-diff output into numbered
//! lines so tests can feed fixtures without a repository.

use std::path::Path;

use crate::git::{ChangeStatus, FileChange, git, git_diff};

/// Most lines kept per file — a huge generated diff can't flood the panel.
const MAX_DIFF_LINES: usize = 400;

/// One rendered row of a file diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// Line number on the old side — `None` for added lines and hunk headers.
    pub old: Option<u32>,
    /// Line number on the new side — `None` for removed lines and headers.
    pub new: Option<u32>,
    /// Line content without the leading `+`/`-`/` ` marker.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    /// `@@ … @@` hunk header.
    Hunk,
    Context,
    Added,
    Removed,
}

/// The parsed unified diff for one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub lines: Vec<DiffLine>,
    /// True when the raw diff exceeded `MAX_DIFF_LINES` and was cut off.
    pub truncated: bool,
}

/// Working-tree diff for `change` under `dir`. `None` only when git itself
/// can't run; a file with no textual diff (binary, mode-only, vanished)
/// yields an empty `FileDiff`.
pub(crate) fn diff_for_file(dir: &Path, change: &FileChange) -> Option<FileDiff> {
    let raw = if change.status == ChangeStatus::Added && !tracked(dir, &change.path) {
        // Untracked files have no index entry — diff against /dev/null.
        let abs = dir.join(&change.path);
        git_diff(dir, &["diff", "--no-index", "--", "/dev/null", &abs.to_string_lossy()])
    } else {
        // `diff HEAD` covers staged + unstaged in one output. On an unborn
        // HEAD (no commits yet) it fails, so fall back to both halves.
        git_diff(dir, &["diff", "HEAD", "--", &change.path]).or_else(|| {
            let mut both = git_diff(dir, &["diff", "--cached", "--", &change.path]).unwrap_or_default();
            both.push_str(&git_diff(dir, &["diff", "--", &change.path]).unwrap_or_default());
            Some(both)
        })
    }?;
    Some(parse_diff(&raw))
}

/// Whether `path` has an index entry — untracked files need `--no-index`.
fn tracked(dir: &Path, path: &str) -> bool {
    git(dir, &["ls-files", "--error-unmatch", "--", path]).is_some()
}

/// Parse unified-diff output into numbered lines. File headers (`diff --git`,
/// `index`, `---`/`+++`, mode lines) are skipped; hunk headers seed the old/new
/// line counters that ` `/`+`/`-` lines then advance. A malformed `@@` line
/// still renders as a hunk row — content is never dropped.
pub(crate) fn parse_diff(raw: &str) -> FileDiff {
    let mut diff = FileDiff::default();
    let mut in_hunk = false;
    let (mut old, mut new) = (0u32, 0u32);
    for line in raw.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            if let Some((o, n)) = hunk_starts(line) {
                old = o;
                new = n;
            }
            diff.lines.push(DiffLine {
                kind: DiffLineKind::Hunk,
                old: None,
                new: None,
                text: line.to_string(),
            });
        } else if !in_hunk {
            continue; // file header lines
        } else if let Some(text) = line.strip_prefix('+') {
            diff.lines.push(DiffLine {
                kind: DiffLineKind::Added,
                old: None,
                new: Some(new),
                text: text.to_string(),
            });
            new += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            diff.lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                old: Some(old),
                new: None,
                text: text.to_string(),
            });
            old += 1;
        } else if line.starts_with('\\') {
            // "\ No newline at end of file" — describes the previous line.
            diff.lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old: None,
                new: None,
                text: line.to_string(),
            });
        } else {
            // Context line (' ' prefix) or a bare empty line git emits for an
            // empty context line.
            let text = line.strip_prefix(' ').unwrap_or(line);
            diff.lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old: Some(old),
                new: Some(new),
                text: text.to_string(),
            });
            old += 1;
            new += 1;
        }
        if diff.lines.len() >= MAX_DIFF_LINES {
            diff.truncated = true;
            break;
        }
    }
    diff
}

/// `@@ -<old>[,n] +<new>[,n] @@` → the two starting line numbers.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut spans = header.split_whitespace().filter(|s| s.starts_with('-') || s.starts_with('+'));
    let old = spans.next()?[1..].split(',').next()?.parse().ok()?;
    let new = spans.next()?[1..].split(',').next()?.parse().ok()?;
    Some((old, new))
}
