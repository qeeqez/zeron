//! Line-level diffs for the Changes panel. `diff_for_file` shells out to git
//! for one changed path; `parse_diff` turns unified-diff output into numbered
//! lines so tests can feed fixtures without a repository.

use std::path::Path;

use crate::git::{ChangeStatus, FileChange, git, git_diff};

/// Most lines kept per file — a huge generated diff can't flood the panel.
const MAX_DIFF_LINES: usize = 400;

/// Most stdout bytes read from `git diff` — bounds the subprocess buffer
/// before `parse_diff` applies its own row cap.
const MAX_DIFF_BYTES: u64 = 512 * 1024;

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

/// Resolve a `ReviewTarget` against the change list: the file's path plus
/// the diff line's number and text, as a `ReviewComment` with empty `text`.
/// `None` when the row has no diff or the line carries no line number
/// (hunk headers, "\ No newline" markers) — those rows aren't commentable.
pub(crate) fn review_anchor(changes: &[FileChange], target: crate::model::ReviewTarget) -> Option<crate::model::ReviewComment> {
    let change = changes.get(target.file_ix)?;
    let line = change.diff.as_ref()?.lines.get(target.line_ix)?;
    // Prefer the new side; a removed line anchors on the old side instead.
    let (number, old_side) = line.new.map(|n| (n, false)).or_else(|| line.old.map(|n| (n, true)))?;
    Some(crate::model::ReviewComment {
        path: change.path.clone(),
        line: number,
        old_side,
        code: line.text.clone(),
        text: String::new(),
    })
}

/// Working-tree diff for `change` under `dir`. `None` only when git itself
/// can't run; a file with no textual diff (binary, mode-only, vanished)
/// yields an empty `FileDiff`.
pub(crate) fn diff_for_file(dir: &Path, change: &FileChange) -> Option<FileDiff> {
    let (raw, capped) = if change.status == ChangeStatus::Added && !tracked(dir, &change.path) {
        // Untracked files have no index entry — diff against /dev/null.
        let abs = dir.join(&change.path);
        git_diff(dir, &["diff", "--no-index", "--", "/dev/null", &abs.to_string_lossy()], MAX_DIFF_BYTES)?
    } else {
        // `diff HEAD` covers staged + unstaged in one output. A rename needs
        // both names in the pathspec — the source alone is gone from the
        // worktree, the destination alone diffs as a new file. On an unborn
        // HEAD (no commits yet) `diff HEAD` fails, so diff the worktree once
        // against the empty tree for the same net result — concatenating the
        // staged and unstaged halves would feed the second patch's headers
        // to `parse_diff` as content.
        git_diff(dir, &diff_args("HEAD", change), MAX_DIFF_BYTES)
            .or_else(|| git_diff(dir, &diff_args(&crate::git::empty_tree_id(dir)?, change), MAX_DIFF_BYTES))?
    };
    let mut diff = parse_diff(&raw);
    diff.truncated |= capped;
    Some(diff)
}

/// Whether `path` has an index entry — untracked files need `--no-index`.
fn tracked(dir: &Path, path: &str) -> bool {
    git(dir, &["ls-files", "--error-unmatch", "--", path]).is_some()
}

/// `git diff <base> -- <path> [source]` — a rename needs both names in the
/// pathspec (the source alone is gone from the worktree, the destination
/// alone diffs as a new file).
fn diff_args<'a>(base: &'a str, change: &'a FileChange) -> Vec<&'a str> {
    let mut args = vec!["diff", base, "--", change.path.as_str()];
    if let Some(source) = &change.source {
        args.push(source.as_str());
    }
    args
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
