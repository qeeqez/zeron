//! `parse_diff` — unified-diff text → `FileDiff` — split from
//! `changes_diff.rs` for the SLOC cap; re-exported there so callers keep
//! using `crate::changes_diff::parse_diff`.

use crate::changes_diff::{DiffLine, DiffLineKind, FileDiff, MAX_DIFF_LINES};

/// Parse unified-diff output into numbered lines. File headers (`diff --git`,
/// `index`, `---`/`+++`, mode lines) are skipped; hunk headers seed the old/new
/// line counters that ` `/`+`/`-` lines then advance. A malformed `@@` line
/// still renders as a hunk row — content is never dropped.
///
/// Hunk byte ranges are recorded against `raw` for the first file block
/// only, so `hunk_patch` can rebuild a single-hunk patch; a second `diff
/// --git` line ends range recording (its hunks still render). Ranges are
/// pushed when a hunk closes — the next `@@`, a new file header, or end of
/// input — so a hunk cut by `MAX_DIFF_LINES` keeps no range and can't be
/// staged half-applied.
pub(crate) fn parse_diff(raw: &str) -> FileDiff {
    let mut diff = FileDiff { raw: raw.to_string(), ..FileDiff::default() };
    let mut in_hunk = false;
    let mut in_first_file = true;
    let mut hunk_start = 0usize;
    let (mut old, mut new) = (0u32, 0u32);
    let mut offset = 0usize;
    for chunk in raw.split_inclusive('\n') {
        let end = offset + chunk.len();
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with("diff --git") {
            // A new file block: close the open hunk and stop recording —
            // only the first block's hunks are stageable.
            if in_hunk && in_first_file {
                diff.hunks.push(hunk_start..offset);
            }
            in_hunk = false;
            if offset != 0 {
                in_first_file = false;
            }
        } else if line.starts_with("@@") {
            if in_hunk && in_first_file {
                diff.hunks.push(hunk_start..offset);
            }
            in_hunk = true;
            hunk_start = offset;
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
            offset = end;
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
        offset = end;
        if diff.lines.len() >= MAX_DIFF_LINES {
            diff.truncated = true;
            return diff;
        }
    }
    if in_hunk && in_first_file {
        diff.hunks.push(hunk_start..raw.len());
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
