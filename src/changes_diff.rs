//! Line-level diffs for the Changes panel. `diff_for_file` shells out to git
//! for one changed path; `parse_diff` turns unified-diff output into numbered
//! lines so tests can feed fixtures without a repository.

use std::path::Path;

use crate::git::{ChangeStatus, FileChange, git_diff, tracked};

/// Intra-line (word-level) highlighting for paired removed/added lines —
/// kept beside the diff model it consumes; `#[path]` because `main.rs` is
/// at the SLOC cap and can't take another `mod`.
#[path = "diff_highlight.rs"]
pub(crate) mod diff_highlight;

/// Most lines kept per file — a huge generated diff can't flood the panel.
pub(crate) const MAX_DIFF_LINES: usize = 400;

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
    /// The raw unified-diff text `lines` was parsed from — hunk patches are
    /// sliced out of it verbatim so `git apply` sees byte-exact context.
    /// Empty for diffs built by hand (tests) instead of `parse_diff`.
    pub raw: String,
    /// Byte ranges into `raw`, one per complete hunk of the first file
    /// block — the `@@` line through the line before the next `@@` (or end
    /// of the block). A hunk cut by `MAX_DIFF_LINES` or belonging to a
    /// second file block gets no range, so it can't be staged.
    pub hunks: Vec<std::ops::Range<usize>>,
}

impl FileDiff {
    /// The single-hunk patch for hunk `ix`: the file's diff header
    /// (`diff --git`/`index`/`---`/`+++`, everything before the first `@@`)
    /// plus that hunk's raw lines. `git apply --cached` needs the header to
    /// know which file the hunk edits. `None` when the hunk has no recorded
    /// range — truncated or from a second file block.
    pub(crate) fn hunk_patch(&self, ix: usize) -> Option<String> {
        let hunk = self.hunks.get(ix)?;
        let header_end = self.hunks.first()?.start;
        let mut patch = self.raw[..header_end].to_string();
        patch.push_str(&self.raw[hunk.clone()]);
        if !patch.ends_with('\n') {
            patch.push('\n');
        }
        Some(patch)
    }
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
/// the diff line's new-side number and text, as a `ReviewComment` with
/// empty `text`. `None` when the row has no diff or the line carries no
/// new-side number — hunk headers, "\ No newline" markers, and removed
/// lines aren't commentable.
pub(crate) fn review_anchor(changes: &[FileChange], target: crate::model::ReviewTarget) -> Option<crate::model::ReviewComment> {
    let change = changes.get(target.file_ix)?;
    let line = change.diff.as_ref()?.lines.get(target.line_ix)?;
    let number = line.new?;
    Some(crate::model::ReviewComment {
        path: change.path.clone(),
        line: number,
        old_side: false,
        code: line.text.clone(),
        text: String::new(),
    })
}

/// Working-tree diff for `change` under `dir`. `None` only when git itself
/// can't run; a file with no textual diff (binary, mode-only, vanished)
/// yields an empty `FileDiff`. `ignore_ws` passes `--ignore-all-space` so
/// whitespace-only edits (reindents, tab↔space) collapse to no diff.
///
/// The row's `staged` flag picks which half of a partially-staged file to
/// show — `git diff --cached` (HEAD→index) when staged, `git diff`
/// (index→worktree) when not — so a hunk's patch always applies against the
/// side the button acts on: `apply --cached` for unstaged hunks,
/// `apply --cached --reverse` for staged ones.
pub(crate) fn diff_for_file(dir: &Path, change: &FileChange, ignore_ws: bool) -> Option<FileDiff> {
    diff_for_file_at(dir, change, None, ignore_ws)
}

/// `diff_for_file` with an explicit base commit — the worktree diff-base
/// mode. `git diff <base> -- <path>` covers the file's whole delta against
/// the base (committed and uncommitted alike), so the `staged` split
/// doesn't apply. Files untracked in the worktree still diff against
/// /dev/null — a base diff can't see them.
pub(crate) fn diff_for_file_at(dir: &Path, change: &FileChange, base: Option<&str>, ignore_ws: bool) -> Option<FileDiff> {
    let (raw, capped) = if change.status == ChangeStatus::Added && !tracked(dir, &change.path) {
        // Untracked files have no index entry — diff against /dev/null.
        let abs = dir.join(&change.path);
        git_diff(dir, &["diff", "--no-index", "--", "/dev/null", &abs.to_string_lossy()], MAX_DIFF_BYTES)?
    } else if let Some(base) = base {
        let mut args = vec!["diff"];
        if ignore_ws {
            args.push("--ignore-all-space");
        }
        args.push(base);
        args.extend(["--", change.path.as_str()]);
        if let Some(source) = &change.source {
            args.push(source.as_str());
        }
        git_diff(dir, &args, MAX_DIFF_BYTES)?
    } else {
        git_diff(dir, &diff_args(change, ignore_ws), MAX_DIFF_BYTES)?
    };
    let mut diff = parse_diff(&raw);
    diff.truncated |= capped;
    Some(diff)
}

/// `git diff [--ignore-all-space] [--cached] -- <path> [source]` — the
/// staged flag picks `--cached` (HEAD→index) over the default
/// index→worktree diff. A rename needs both names in the pathspec (the
/// source alone is gone from the worktree, the destination alone diffs as
/// a new file). Both forms work on an unborn HEAD: `--cached` diffs
/// against the implicit empty tree, and the index always exists.
fn diff_args(change: &FileChange, ignore_ws: bool) -> Vec<&str> {
    let mut args = vec!["diff"];
    if ignore_ws {
        args.push("--ignore-all-space");
    }
    if change.staged {
        args.push("--cached");
    }
    args.extend(["--", change.path.as_str()]);
    if let Some(source) = &change.source {
        args.push(source.as_str());
    }
    args
}

/// `parse_diff` — unified-diff text → `FileDiff` — split into
/// `changes_diff_parse.rs` for the SLOC cap; re-exported so callers keep
/// using `crate::changes_diff::parse_diff`.
#[path = "changes_diff_parse.rs"]
pub(crate) mod parse;
pub(crate) use parse::parse_diff;

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
