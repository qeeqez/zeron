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

/// How the expanded diff lays out its lines — the Changes panel's view-mode
/// toggle. Persisted as `Settings.diff_mode` via `name`/`from_name`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    /// One column: removed and added lines interleaved in unified order.
    #[default]
    Unified,
    /// Two columns: old on the left, new on the right, aligned by line.
    Split,
}

impl DiffMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unified => "unified",
            Self::Split => "split",
        }
    }

    /// Anything unrecognized (including an empty legacy value) is Unified.
    pub fn from_name(name: &str) -> Self {
        match name {
            "split" => Self::Split,
            _ => Self::Unified,
        }
    }
}

/// One row of the split layout: either a full-width line (hunk header) or
/// an old|new cell pair. Values are indexes into `FileDiff::lines`, so
/// review anchors keep resolving against the same
/// `ReviewTarget { file_ix, line_ix }` the unified view uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitRow {
    /// A line with no side — hunk headers span both columns.
    Wide(usize),
    /// Old-side line index | new-side line index. Context lines put the same
    /// index in both cells; a `None` cell is the empty side of an unpaired
    /// removal or addition.
    Pair { old: Option<usize>, new: Option<usize> },
}

impl SplitRow {
    /// Whether this row renders `line_ix` in either cell — used to place the
    /// comment editor under the row its anchor lives in.
    pub fn contains(&self, line_ix: usize) -> bool {
        match self {
            Self::Wide(ix) => *ix == line_ix,
            Self::Pair { old, new } => *old == Some(line_ix) || *new == Some(line_ix),
        }
    }
}

/// Lay `diff` out as split rows: each run of removed lines pairs with the
/// run of added lines that follows it (GitHub-style — the first removed
/// line sits across from the first added line, leftovers get an empty
/// opposite cell). Context lines span both columns; hunk headers are
/// full-width rows; "\ No newline" markers ride the side they describe.
pub(crate) fn split_rows(diff: &FileDiff) -> Vec<SplitRow> {
    /// Emit the pending removed/added runs as paired rows — the first
    /// removed line sits across from the first added line, leftovers get an
    /// empty opposite cell — then the buffered "\ No newline" markers, each
    /// on the side of the line it describes.
    fn flush(
        removed: &mut Vec<usize>, added: &mut Vec<usize>, old_marks: &mut Vec<usize>, new_marks: &mut Vec<usize>, rows: &mut Vec<SplitRow>,
    ) {
        let n = removed.len().max(added.len());
        for i in 0..n {
            rows.push(SplitRow::Pair { old: removed.get(i).copied(), new: added.get(i).copied() });
        }
        rows.extend(old_marks.drain(..).map(|ix| SplitRow::Pair { old: Some(ix), new: None }));
        rows.extend(new_marks.drain(..).map(|ix| SplitRow::Pair { old: None, new: Some(ix) }));
        removed.clear();
        added.clear();
    }

    let mut rows = Vec::with_capacity(diff.lines.len());
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    // Markers buffer on the side of the line they describe so they don't
    // split a removed+added run's pairing.
    let (mut old_marks, mut new_marks) = (Vec::new(), Vec::new());
    let mut last_kind = None;
    for (ix, line) in diff.lines.iter().enumerate() {
        match line.kind {
            DiffLineKind::Removed => {
                removed.push(ix);
                last_kind = Some(DiffLineKind::Removed);
            },
            DiffLineKind::Added => {
                added.push(ix);
                last_kind = Some(DiffLineKind::Added);
            },
            DiffLineKind::Context if line.old.is_none() => match last_kind {
                Some(DiffLineKind::Removed) => old_marks.push(ix),
                Some(DiffLineKind::Added) => new_marks.push(ix),
                // After a context line the marker describes both sides.
                _ => rows.push(SplitRow::Wide(ix)),
            },
            // A numbered context line pairs with itself; a hunk header is a
            // wide row. Either way the pending runs end here and pair off.
            DiffLineKind::Context | DiffLineKind::Hunk => {
                flush(&mut removed, &mut added, &mut old_marks, &mut new_marks, &mut rows);
                last_kind = Some(line.kind);
                rows.push(if line.old.is_some() {
                    SplitRow::Pair { old: Some(ix), new: Some(ix) }
                } else {
                    SplitRow::Wide(ix)
                });
            },
        }
    }

    flush(&mut removed, &mut added, &mut old_marks, &mut new_marks, &mut rows);
    rows
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
/// yields an empty `FileDiff`. `ignore_ws` passes `--ignore-all-space` so
/// whitespace-only edits (reindents, tab↔space) collapse to no diff.
pub(crate) fn diff_for_file(dir: &Path, change: &FileChange, ignore_ws: bool) -> Option<FileDiff> {
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
        git_diff(dir, &diff_args("HEAD", change, ignore_ws), MAX_DIFF_BYTES)
            .or_else(|| git_diff(dir, &diff_args(&crate::git::empty_tree_id(dir)?, change, ignore_ws), MAX_DIFF_BYTES))?
    };
    let mut diff = parse_diff(&raw);
    diff.truncated |= capped;
    Some(diff)
}

/// Whether `path` has an index entry — untracked files need `--no-index`.
fn tracked(dir: &Path, path: &str) -> bool {
    git(dir, &["ls-files", "--error-unmatch", "--", path]).is_some()
}

/// `git diff [--ignore-all-space] <base> -- <path> [source]` — a rename
/// needs both names in the pathspec (the source alone is gone from the
/// worktree, the destination alone diffs as a new file).
fn diff_args<'a>(base: &'a str, change: &'a FileChange, ignore_ws: bool) -> Vec<&'a str> {
    let mut args = vec!["diff"];
    if ignore_ws {
        args.push("--ignore-all-space");
    }
    args.extend([base, "--", change.path.as_str()]);
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

/// The header's diff-stat rollup — files touched plus summed insertions and
/// deletions across every row. `None` on a clean tree so the line hides.
pub(crate) struct DiffSummary {
    pub files: usize,
    pub added: u32,
    pub deleted: u32,
}

impl DiffSummary {
    /// `K files changed · +N −M` — the row's aria label and the string tests
    /// assert on.
    pub(crate) fn text(&self) -> String {
        format!("{} files changed · +{} −{}", self.files, self.added, self.deleted)
    }
}

/// Sum the per-file numstat counts into the header's summary. Untracked
/// files already carry their line count in `added` (see `git::collect`), so
/// they fold in like any other row.
pub(crate) fn diff_summary(changes: &[FileChange]) -> Option<DiffSummary> {
    if changes.is_empty() {
        return None;
    }
    Some(DiffSummary {
        files: changes.len(),
        added: changes.iter().map(|c| c.added).sum(),
        deleted: changes.iter().map(|c| c.deleted).sum(),
    })
}
