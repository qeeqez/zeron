//! Per-line blame and per-file history for the file menu's "Blame" and
//! "File History" items — `git blame --porcelain` and `git log --follow`
//! shell-outs plus their parsers. Split from `git.rs` for the SLOC cap;
//! re-exported there so callers keep using `crate::git::blame` /
//! `crate::git::file_log`.

use std::collections::HashMap;
use std::path::Path;

use super::{Commit, CommitDiff, MAX_SHOW_BYTES};

/// One line of `git blame` output as the overlay's monospace list sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Final line number (1-based) — the row's gutter.
    pub line_no: u32,
    /// Full commit id that last touched the line — the row shows its short
    /// prefix and copies the whole hash on click.
    pub sha: String,
    /// Author name (`author` header).
    pub author: String,
    /// Commit subject (`summary` header).
    pub summary: String,
    /// The line's text, without the trailing newline.
    pub text: String,
}

/// `git blame --porcelain -- <path>` under `dir`, one `BlameLine` per file
/// line. `Err` when git fails — an untracked or absent path lands as git's
/// own stderr, which the overlay shows.
pub(crate) fn blame(dir: &Path, path: &str) -> Result<Vec<BlameLine>, String> {
    let out = super::git_env(dir, &["blame", "--porcelain", "--", path], &[])?;
    Ok(parse_blame(&out))
}

/// `git log --follow --format=… -- <path>` — every commit that touched the
/// file, newest first, across renames. Reuses the Changes panel's `Commit`
/// and `parse_log`; `Err` when git fails (non-repo, bad path).
pub(crate) fn file_log(dir: &Path, path: &str) -> Result<Vec<Commit>, String> {
    let out = super::git_env(dir, &["log", "--follow", "--format=%h%x00%s%x00%an%x00%ar", "--", path], &[])?;
    Ok(crate::git_parse::parse_log(&out))
}

/// `git show --format= <sha> -- <path>` — the commit's patch for one file.
/// `Err` when git can't run or `sha` doesn't resolve; an empty patch (the
/// file went by another name in that commit) parses to an empty `CommitDiff`.
pub(crate) fn commit_file_diff(dir: &Path, sha: &str, path: &str) -> Result<CommitDiff, String> {
    let (raw, capped) =
        super::git_diff(dir, &["show", "--format=", sha, "--", path], MAX_SHOW_BYTES).ok_or_else(|| format!("git show {sha} failed"))?;
    let mut diff = crate::git_parse::parse_commit_diff(&raw);
    diff.truncated |= capped;
    Ok(diff)
}

/// Parse `git blame --porcelain`: each record is a header line
/// `<sha> <orig> <final> [count]`, attribute lines (`author`, `summary`, …),
/// then the line text prefixed with a tab. Attributes print only on a
/// commit's first record, so they're cached per sha — a later record for the
/// same commit reuses them.
fn parse_blame(raw: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    let mut attrs: HashMap<String, (String, String)> = HashMap::new();
    let mut pending: Option<(String, u32)> = None;
    for line in raw.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            if let Some((sha, line_no)) = pending.take() {
                let (author, summary) = attrs.get(&sha).cloned().unwrap_or_default();
                lines.push(BlameLine { line_no, sha, author, summary, text: text.to_string() });
            }
        } else if let Some(rest) = line.strip_prefix("author ") {
            if let Some((sha, _)) = &pending {
                attrs.entry(sha.clone()).or_default().0 = rest.to_string();
            }
        } else if let Some(rest) = line.strip_prefix("summary ") {
            if let Some((sha, _)) = &pending {
                attrs.entry(sha.clone()).or_default().1 = rest.to_string();
            }
        } else if let Some((sha, no)) = header(line) {
            pending = Some((sha, no));
        }
    }
    lines
}

/// A blame record header: `<sha> <orig-line> <final-line> [count]` — the sha
/// is a hex object id (SHA-1 or SHA-256), which keeps attribute keys like
/// `author` from parsing as headers.
fn header(line: &str) -> Option<(String, u32)> {
    let mut f = line.split(' ');
    let sha = f.next()?;
    if sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    f.next()?; // original line number — the overlay shows final numbers
    let line_no = f.next()?.parse().ok()?;
    Some((sha.to_string(), line_no))
}

#[cfg(test)]
#[path = "blame_tests.rs"]
mod blame_tests;

#[cfg(test)]
#[path = "blame_ui_tests.rs"]
mod blame_ui_tests;
