//! Text scans over the terminal's rendered contents: `find_in_lines` backs
//! the Cmd-F find bar, `detect_links` finds the clickable spans (URLs and
//! file paths that exist on disk). Both work on `TermSession::contents()`
//! output — logical lines, ANSI already resolved — and report byte ranges
//! into that string, which is what `StyledText` highlights and
//! `InteractiveText` click ranges address.

use std::ops::Range;
use std::path::{Component, Path};

/// One search hit in the rendered contents: the logical line index plus the
/// byte range of the match inside the whole contents string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TermMatch {
    pub line: usize,
    pub range: Range<usize>,
}

/// A clickable span in the rendered contents: a URL or a file path that
/// exists on disk. `target` is what the click acts on — the URL itself, or
/// the path to mention (project-relative when it lives under the root).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TermLink {
    pub range: Range<usize>,
    pub target: String,
    pub is_url: bool,
}

/// Case-insensitive search over `contents` split into logical lines. Empty
/// queries match nothing; matches never overlap and never span a newline.
pub(crate) fn find_in_lines(contents: &str, query: &str) -> Vec<TermMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    let mut base = 0;
    for (line_ix, line) in contents.split('\n').enumerate() {
        let mut start = 0;
        while let Some(hit) = match_at(line, start, query) {
            matches.push(TermMatch { line: line_ix, range: base + hit.start..base + hit.end });
            // `hit` is never empty (query isn't), so `end` strictly advances.
            start = hit.end;
        }
        base += line.len() + 1;
    }
    matches
}

/// First occurrence of `query` at or after `start` in `hay`, compared
/// char-by-char on lowercase so byte offsets stay valid for highlighting.
fn match_at(hay: &str, start: usize, query: &str) -> Option<Range<usize>> {
    hay.char_indices()
        .skip_while(|(ix, _)| *ix < start)
        .take_while(|(ix, _)| hay.len() - ix >= query.len())
        .find_map(|(ix, _)| match_prefix(&hay[ix..], query).map(|len| ix..ix + len))
}

/// Bytes of `hay`'s leading run that case-insensitively equals `query` —
/// `None` when the prefix doesn't match. Char-wise so a multi-char
/// lowercase (e.g. İ → i̇) still lines up.
fn match_prefix(hay: &str, query: &str) -> Option<usize> {
    let mut len = 0;
    let mut hs = hay.chars();
    for q in query.chars() {
        match hs.next() {
            Some(h) if h.to_lowercase().eq(q.to_lowercase()) => len += h.len_utf8(),
            _ => return None,
        }
    }
    Some(len)
}

/// Scan the logical lines in `visible` for `http(s)://` URLs and file paths
/// that exist on disk. `exists` receives the candidate path — relative
/// candidates are already joined onto `root` — and answers whether to link
/// it. Ranges are byte offsets into `contents`, sorted and non-overlapping.
pub(crate) fn detect_links(contents: &str, visible: Range<usize>, root: &Path, mut exists: impl FnMut(&Path) -> bool) -> Vec<TermLink> {
    let mut links = Vec::new();
    let mut base = 0;
    for (line_ix, line) in contents.split('\n').enumerate() {
        if visible.contains(&line_ix) {
            scan_line(line, base, root, &mut exists, &mut links);
        }
        base += line.len() + 1;
    }
    links
}

/// One line's tokens, whitespace-split: URLs first (they win the span), then
/// path candidates that survive `exists`.
fn scan_line(line: &str, base: usize, root: &Path, exists: &mut impl FnMut(&Path) -> bool, links: &mut Vec<TermLink>) {
    for (start, raw) in tokens(line) {
        if let Some(pos) = raw.find("http://").or_else(|| raw.find("https://")) {
            let url = raw[pos..].trim_end_matches(TRAIL);
            if url.len() > "https://".len() {
                links.push(TermLink {
                    range: base + start + pos..base + start + pos + url.len(),
                    target: url.to_string(),
                    is_url: true,
                });
            }
            continue;
        }
        let tok = raw.trim_matches(TRIM);
        // Path-shaped only — a `/` or `.` keeps plain words (and flags)
        // from ever reaching the filesystem.
        if tok.is_empty() || !tok.contains(['/', '.']) || !tok.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        let path_part = strip_line_col(tok);
        if path_part.is_empty() {
            continue;
        }
        let rel = Path::new(path_part);
        if rel.components().any(|c| matches!(c, Component::ParentDir)) {
            continue;
        }
        let (abs, target) = if rel.is_absolute() {
            let target = rel.strip_prefix(root).map_or(path_part, |r| r.to_str().unwrap_or(path_part)).to_string();
            (rel.to_path_buf(), target)
        } else {
            (root.join(rel), path_part.to_string())
        };
        if exists(&abs) {
            // `tok` is `raw` minus its trimmed ends — `find` lands on the
            // interior occurrence, never inside the leading trim run.
            let off = raw.find(tok).unwrap_or(0);
            links.push(TermLink {
                range: base + start + off..base + start + off + tok.len(),
                target,
                is_url: false,
            });
        }
    }
}

/// Whitespace-separated tokens with their byte offset in the line.
fn tokens(line: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut ix = 0;
    std::iter::from_fn(move || {
        let rest = line.get(ix..)?;
        let tok = rest.split_whitespace().next()?;
        let start = ix + rest.len() - rest.trim_start().len();
        ix = start + tok.len();
        Some((start, tok))
    })
}

/// Punctuation that wraps a path in prose — trimmed from both ends. `:` is
/// included so `error: src/x.rs` still links, and `src/x.rs:` drops its colon.
const TRIM: &[char] = &['"', '\'', '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', ':'];
/// Trailing punctuation only — URLs keep interior `)`/`,` but not a closer.
const TRAIL: &[char] = &['"', '\'', ')', ']', '}', '>', ',', ';', '.', '!', '?', ':'];

/// `src/x.rs:12:4` → `src/x.rs` — drop a trailing `:line` or `:line:col`.
fn strip_line_col(tok: &str) -> &str {
    let mut s = tok;
    for _ in 0..2 {
        match s.rsplit_once(':') {
            Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => s = head,
            _ => break,
        }
    }
    s
}
