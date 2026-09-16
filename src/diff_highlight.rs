//! Word-level (intra-line) highlighting for expanded diffs. `changed_spans`
//! finds the byte range that differs between a removed line and the added
//! line it pairs with; `line_spans` maps those ranges over a whole diff using
//! the same removed↔added pairing `split_rows` lays out; `MarkedDiff` carries
//! the result into the unified and split renderers, which paint the changed
//! range as a stronger wash of the row's tint.

use gpui_kit::*;

use crate::changes_diff::{DiffLineKind, FileDiff, SplitRow, split_rows};

/// Byte ranges within one line — `(start, end)` pairs on char boundaries.
pub type ByteSpans = Vec<(usize, usize)>;

/// Byte ranges that differ between `old` and `new` — one `(start, end)` span
/// per side, empty when the lines are identical. Common leading and trailing
/// chars stay unmarked; everything between them is the change. A pure
/// insertion marks an empty range at the insertion point on the old side.
/// Ranges always fall on char boundaries so they can feed `StyledText`
/// highlights directly.
pub fn changed_spans(old: &str, new: &str) -> (ByteSpans, ByteSpans) {
    if old == new {
        return (Vec::new(), Vec::new());
    }
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(old, new, prefix);
    (vec![(prefix, old.len() - suffix)], vec![(prefix, new.len() - suffix)])
}

/// Bytes `a` and `b` share from the start, counted in whole chars.
fn common_prefix(a: &str, b: &str) -> usize {
    let mut len = 0;
    for ((ai, ca), (bi, cb)) in a.char_indices().zip(b.char_indices()) {
        if ai != bi || ca != cb {
            break;
        }
        len = ai + ca.len_utf8();
    }
    len
}

/// Bytes `a` and `b` share from the end, counted in whole chars and never
/// reaching back into the common prefix — the changed middle can't overlap
/// itself when one side is a prefix of the other.
fn common_suffix(a: &str, b: &str, prefix: usize) -> usize {
    let mut len = 0;
    for ((ai, ca), (bi, cb)) in a.char_indices().rev().zip(b.char_indices().rev()) {
        if ai < prefix || bi < prefix || ca != cb {
            break;
        }
        len += ca.len_utf8();
    }
    len
}

/// Per-line changed ranges for every line of `diff`, indexed by line index —
/// an empty `Vec` for lines with no intra-line mark (context, hunk headers,
/// unpaired removals/additions). Removed and added runs pair index-wise, the
/// same alignment `split_rows` gives the split view, so both layouts mark the
/// same regions.
pub(crate) fn line_spans(diff: &FileDiff) -> Vec<ByteSpans> {
    let mut marks = vec![Vec::new(); diff.lines.len()];
    for row in split_rows(diff) {
        let SplitRow::Pair { old: Some(old), new: Some(new) } = row else { continue };
        let (old_line, new_line) = (&diff.lines[old], &diff.lines[new]);
        if old_line.kind != DiffLineKind::Removed || new_line.kind != DiffLineKind::Added {
            continue;
        }
        let (old_spans, new_spans) = changed_spans(&old_line.text, &new_line.text);
        marks[old] = old_spans;
        marks[new] = new_spans;
    }
    marks
}

/// A diff plus its per-line changed ranges — `None` marks when the
/// ignore-whitespace filter is on (whitespace-only edits would mark nothing
/// meaningful, and the flag exists to hide exactly that noise).
pub(crate) struct MarkedDiff<'a> {
    pub diff: &'a FileDiff,
    marks: Vec<ByteSpans>,
}

impl<'a> MarkedDiff<'a> {
    pub fn new(diff: &'a FileDiff, highlight: bool) -> Self {
        Self {
            diff,
            marks: if highlight { line_spans(diff) } else { Vec::new() },
        }
    }

    /// The line's code text: plain when it carries no mark, a `StyledText`
    /// with the changed range washed in `fg` at a stronger alpha when it
    /// does. The caller's wrapper div still owns color and truncation — the
    /// highlight only adds a background.
    pub fn code_text(&self, line_ix: usize, fg: Hsla) -> AnyElement {
        let line = &self.diff.lines[line_ix];
        match self.marks.get(line_ix).map(Vec::as_slice) {
            // An empty range — the change sits entirely on the other side —
            // has nothing to paint, so it falls through to plain text.
            Some(&[span]) if span.0 < span.1 => {
                let mark = HighlightStyle {
                    background_color: Some(fg.opacity(0.3)),
                    ..Default::default()
                };
                StyledText::new(line.text.clone()).with_highlights([(span.0..span.1, mark)]).into_any_element()
            },
            _ => line.text.clone().into_any_element(),
        }
    }
}

#[cfg(test)]
#[path = "diff_highlight_tests.rs"]
mod diff_highlight_tests;
